#!/usr/bin/env python3
"""RSocket inference service with HTTP-only health and process metrics."""
import asyncio
import json
import os
import resource
import threading
from aiohttp import web
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from rsocket.payload import Payload
from rsocket.extensions.mimetypes import WellKnownMimeTypes
from rsocket.request_handler import BaseRequestHandler
from rsocket.rsocket_server import RSocketServer
from rsocket.transports.aiohttp_websocket import TransportAioHttpWebsocket

MODEL_ID = "burakutf/finetuned-finbert-crypto"
MODEL_REVISION = "24ca955d09a693342a25e6a22249346431cd778c"
torch = None
tokenizer = None
model = None


def classify(articles):
    texts = [item["title"] + chr(10) * 2 + item.get("article_text", "") for item in articles]
    encoded = tokenizer(texts, return_tensors="pt", padding=True, truncation=True, max_length=256)
    with torch.inference_mode():
        probabilities = torch.softmax(model(**encoded).logits, dim=-1)
    # Model card class order: 0 negative, 1 neutral, 2 positive.
    results = []
    for row in probabilities:
        negative, neutral, positive = (float(value) for value in row.tolist())
        score = positive - negative
        direction = "bullish" if score >= 0.05 else "bearish" if score <= -0.05 else "neutral"
        results.append({"direction": direction, "score": score, "confidence": max(negative, neutral, positive)})
    return results


class OpsHandler(BaseHTTPRequestHandler):
    def respond(self, status, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/process-cpu-seconds":
            usage = resource.getrusage(resource.RUSAGE_SELF)
            body = str(usage.ru_utime + usage.ru_stime).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path == "/process-memory-bytes":
            try:
                with open("/proc/self/status", encoding="utf-8") as status_file:
                    resident_kb = next(int(line.split()[1]) for line in status_file if line.startswith("VmRSS:"))
                body = str(resident_kb * 1024).encode("utf-8")
            except (OSError, StopIteration, ValueError):
                body = b"0"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path != "/health":
            self.respond(404, {"error": "not found"})
            return
        self.respond(200, {"status": "ok", "model": MODEL_ID})

    def log_message(self, format_string, *args):
        print(f"sentiment-service: {format_string % args}", flush=True)


class SentimentRequestHandler(BaseRequestHandler):
    async def request_response(self, payload):
        articles = decode_request(payload.data or b"")
        loop = asyncio.get_running_loop()
        result = await loop.run_in_executor(None, classify, articles)
        response = loop.create_future()
        response.set_result(Payload(data=json.dumps(result).encode("utf-8")))
        return response


def decode_request(raw):
    if not raw or len(raw) > 2_000_000:
        raise ValueError("request body outside allowed size")
    articles = json.loads(raw)
    if not isinstance(articles, list) or len(articles) > 50:
        raise ValueError("expected at most 50 article objects")
    if any(not isinstance(item, dict) or not isinstance(item.get("title"), str) for item in articles):
        raise ValueError("each item must have a title")
    return articles


async def rsocket_websocket(request):
    websocket = web.WebSocketResponse(max_msg_size=2_000_000)
    await websocket.prepare(request)
    transport = TransportAioHttpWebsocket(websocket)
    RSocketServer(
        transport,
        handler_factory=SentimentRequestHandler,
        data_encoding=WellKnownMimeTypes.APPLICATION_OCTET_STREAM,
        metadata_encoding=WellKnownMimeTypes.TEXT_PLAIN,
    )
    await transport.handle_incoming_ws_messages()
    return websocket


def main():
    global model, tokenizer, torch
    import torch as torch_module
    from transformers import AutoModelForSequenceClassification, AutoTokenizer

    torch = torch_module
    torch.set_num_threads(max(1, int(os.environ.get("NEWS_SENTIMENT_THREADS", "4"))))
    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, revision=MODEL_REVISION)
    model = AutoModelForSequenceClassification.from_pretrained(MODEL_ID, revision=MODEL_REVISION)
    model.eval()
    host = os.environ.get("NEWS_SENTIMENT_HOST", "0.0.0.0")
    ops_port = int(os.environ.get("NEWS_SENTIMENT_OPS_PORT", "3006"))
    rsocket_port = int(os.environ.get("NEWS_SENTIMENT_RSOCKET_PORT", "10012"))
    ops_server = ThreadingHTTPServer((host, ops_port), OpsHandler)
    threading.Thread(target=ops_server.serve_forever, daemon=True).start()
    app = web.Application()
    app.router.add_get("/rsocket", rsocket_websocket)
    print(f"Loaded {MODEL_ID}; ops HTTP on {host}:{ops_port}, RSocket WebSocket on {host}:{rsocket_port}/rsocket", flush=True)
    web.run_app(app, host=host, port=rsocket_port)


if __name__ == "__main__":
    main()
