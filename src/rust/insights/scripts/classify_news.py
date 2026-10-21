#!/usr/bin/env python3
"""Local HTTP service for crypto financial-news sentiment."""
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

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
        direction = "bullish" if score >= 0.15 else "bearish" if score <= -0.15 else "neutral"
        results.append({"direction": direction, "score": score, "confidence": max(negative, neutral, positive)})
    return results


class Handler(BaseHTTPRequestHandler):
    def respond(self, status, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path != "/health":
            self.respond(404, {"error": "not found"})
            return
        self.respond(200, {"status": "ok", "model": MODEL_ID})

    def do_POST(self):
        if self.path != "/classify":
            self.respond(404, {"error": "not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length <= 0 or length > 2_000_000:
                self.respond(413, {"error": "request body outside allowed size"})
                return
            articles = json.loads(self.rfile.read(length))
            if not isinstance(articles, list) or len(articles) > 50:
                self.respond(400, {"error": "expected at most 50 article objects"})
                return
            if any(not isinstance(item, dict) or not isinstance(item.get("title"), str) for item in articles):
                self.respond(400, {"error": "each item must have a title"})
                return
            self.respond(200, classify(articles))
        except Exception as error:
            print(f"classification request failed: {error}", flush=True)
            self.respond(500, {"error": "classification failed"})

    def log_message(self, format_string, *args):
        print(f"sentiment-service: {format_string % args}", flush=True)


def main():
    global model, tokenizer, torch
    import torch as torch_module
    from transformers import AutoModelForSequenceClassification, AutoTokenizer

    torch = torch_module
    torch.set_num_threads(max(1, int(os.environ.get("NEWS_SENTIMENT_THREADS", "4"))))
    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, revision=MODEL_REVISION)
    model = AutoModelForSequenceClassification.from_pretrained(MODEL_ID, revision=MODEL_REVISION)
    model.eval()
    host = os.environ.get("NEWS_SENTIMENT_HOST", "127.0.0.1")
    port = int(os.environ.get("NEWS_SENTIMENT_PORT", "3006"))
    server = ThreadingHTTPServer((host, port), Handler)
    print(f"Loaded {MODEL_ID} and listening on http://{host}:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
