import unittest
from unittest.mock import AsyncMock, patch

import exporter
from rsocket.exceptions import RSocketTransportError


class RSocketExporterTests(unittest.TestCase):
    def test_position_management_route_returns_json_body(self):
        with patch.object(
            exporter,
            "_rsocket_request",
            new=AsyncMock(return_value={"status": 200, "body": {"equity": 42}}),
        ) as request:
            result = exporter.rsocket_get("ws://position-management:10005", "/positions/account")

        self.assertEqual(result, {"equity": 42})
        self.assertEqual(request.await_args.args[1], {"method": "GET", "path": "/positions/account"})

    def test_non_success_route_status_is_not_returned_as_data(self):
        with patch.object(
            exporter,
            "_rsocket_request",
            new=AsyncMock(return_value={"status": 503, "body": {"error": "unavailable"}}),
        ):
            with self.assertRaisesRegex(ValueError, "status 503"):
                exporter.rsocket_get("ws://auto-trader:10006", "/positions/auto-trader")

    def test_learning_worker_route_uses_required_request_envelope(self):
        with patch.object(
            exporter,
            "_rsocket_request",
            new=AsyncMock(
                return_value={
                    "worker_route": {
                        "response": {"status": 200, "body": {"summary": {"all": {"total": 2}}}}
                    }
                }
            ),
        ) as request:
            result = exporter.learning_get("ws://learning:10010", "/outcomes?source=all&page_size=1")

        sent = request.await_args.args[1]
        self.assertEqual(result["summary"]["all"]["total"], 2)
        self.assertEqual(sent["operation"], "worker_route")
        self.assertEqual(sent["request"]["method"], "get")
        self.assertEqual(sent["request"]["path"], "/outcomes?source=all&page_size=1")
        self.assertIsNone(sent["request"]["strategy"])
        self.assertIsNone(sent["request"]["payload"])

    def test_rsocket_requests_reuse_one_persistent_endpoint_client(self):
        endpoint = "ws://test-rsocket-pool:10006"
        factory_calls = []

        class ClientContext:
            def __init__(self):
                self.request_count = 0
                self.closed = False

            async def __aenter__(self):
                return self

            async def __aexit__(self, *_):
                self.closed = True

            async def request_response(self, _payload):
                self.request_count += 1
                status = 503 if self.request_count == 1 else 200
                return type(
                    "Response",
                    (),
                    {"data": b'{"status":' + str(status).encode() + b',"body":{"request_count":' + str(self.request_count).encode() + b'}}'},
                )()

        def context_factory(_endpoint):
            context = ClientContext()
            factory_calls.append(context)
            return context

        with patch.object(exporter, "_websocket_client_context", side_effect=context_factory):
            with self.assertRaisesRegex(ValueError, "status 503"):
                exporter.rsocket_get(endpoint, "/application-error")
            first = exporter.rsocket_get(endpoint, "/first")
            second = exporter.rsocket_get(endpoint, "/second")
            self.assertEqual(first["request_count"], 2)
            self.assertEqual(second["request_count"], 3)
            self.assertEqual(len(factory_calls), 1)
            self.assertFalse(factory_calls[0].closed)

        context, client = exporter._RSOCKET_CLIENTS[endpoint]
        asyncio = __import__("asyncio")
        asyncio.run_coroutine_threadsafe(
            exporter._discard_rsocket_client(endpoint, context, client), exporter._rsocket_loop()
        ).result(timeout=2)
        self.assertTrue(context.closed)

    def test_transport_failure_discards_client_without_retrying_request(self):
        endpoint = "ws://test-rsocket-reconnect:10006"
        contexts = []

        class ClientContext:
            def __init__(self, fail):
                self.fail = fail
                self.request_count = 0
                self.closed = False

            async def __aenter__(self):
                return self

            async def __aexit__(self, *_):
                self.closed = True

            async def request_response(self, _payload):
                self.request_count += 1
                if self.fail:
                    raise RSocketTransportError()
                return type("Response", (), {"data": b'{"status":200,"body":{"ok":true}}'})()

        def context_factory(_endpoint):
            context = ClientContext(fail=not contexts)
            contexts.append(context)
            return context

        with patch.object(exporter, "_websocket_client_context", side_effect=context_factory):
            with self.assertRaisesRegex(OSError, "RSocket request"):
                exporter.rsocket_get(endpoint, "/first")
            self.assertEqual(contexts[0].request_count, 1)
            self.assertTrue(contexts[0].closed)
            self.assertEqual(exporter.rsocket_get(endpoint, "/second"), {"ok": True})
            self.assertEqual(len(contexts), 2)
            self.assertEqual(contexts[1].request_count, 1)

        context, client = exporter._RSOCKET_CLIENTS[endpoint]
        asyncio = __import__("asyncio")
        asyncio.run_coroutine_threadsafe(
            exporter._discard_rsocket_client(endpoint, context, client), exporter._rsocket_loop()
        ).result(timeout=2)


if __name__ == "__main__":
    unittest.main()
