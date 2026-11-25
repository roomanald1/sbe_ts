import unittest
from unittest.mock import patch
from unittest.mock import AsyncMock

import exporter
from rsocket.exceptions import RSocketTransportError


class RSocketExporterTests(unittest.TestCase):
    def test_live_position_counts_use_lightweight_route_and_accept_zero_counts(self):
        with patch.object(
            exporter,
            "rsocket_get",
            return_value={"live-momentum": 3, "live-trade-flow": 0},
        ) as request:
            counts = exporter.strategy_live_position_counts()

        self.assertEqual(request.call_args.args[1], "/internal/strategy/open-position-counts")
        self.assertTrue(counts["live_available"])
        self.assertEqual(counts["live"]["live-momentum"], 3)
        self.assertEqual(counts["live"]["live-trade-flow"], 0)

    def test_live_position_counts_mark_failed_or_malformed_response_unavailable(self):
        with patch.object(exporter, "rsocket_get", side_effect=OSError("route unavailable")):
            failed = exporter.strategy_live_position_counts()
        with patch.object(exporter, "rsocket_get", return_value=[1, 2]):
            malformed = exporter.strategy_live_position_counts()
        with patch.object(
            exporter,
            "rsocket_get",
            return_value={"live-momentum": -1},
        ):
            invalid_count = exporter.strategy_live_position_counts()

        self.assertFalse(failed["live_available"])
        self.assertIn("route unavailable", failed["live_error"])
        self.assertFalse(malformed["live_available"])
        self.assertFalse(invalid_count["live_available"])
        self.assertIn("was invalid", invalid_count["live_error"])

    def test_live_position_counts_ignore_untracked_names_without_changing_metric_contract(self):
        with patch.object(
            exporter,
            "rsocket_get",
            return_value={"live-momentum": 2, "unknown-retired-strategy": 99},
        ):
            counts = exporter.strategy_live_position_counts()

        self.assertTrue(counts["live_available"])
        self.assertEqual(counts["live"]["live-momentum"], 2)
        self.assertNotIn("unknown-retired-strategy", counts["live"])

    def test_live_worker_metrics_expose_position_count_route_failure(self):
        class Response:
            def __enter__(self):
                return self

            def __exit__(self, *_):
                return None

            def read(self):
                return b""

        output = []
        with patch.object(exporter, "urlopen", return_value=Response()):
            exporter.strategy_worker_metrics(
                output, {"live": None, "live_available": False}
            )

        self.assertTrue(any(
            "app_strategy_worker_open_instances" in line
            and 'role="live"' in line
            and 'status="UNAVAILABLE"' in line
            and line.rstrip().endswith("-2")
            for line in output
        ))
        self.assertTrue(any(
            "app_strategy_position_counts_available " in line
            and line.rstrip().endswith(" 0")
            for line in output
        ))

    def test_worker_memory_limit_bytes_supports_compose_units(self):
        with patch.dict("os.environ", {"STRATEGY_LIVE_MEMORY_LIMIT": "256M", "STRATEGY_SCOUT_MEMORY_LIMIT": "1.5G"}):
            self.assertEqual(exporter.worker_memory_limit_bytes("live"), 256 * 1024 * 1024)
            self.assertEqual(exporter.worker_memory_limit_bytes("scout"), int(1.5 * 1024**3))

    def test_strategy_worker_scrape_covers_both_pools_without_symbol_labels(self):
        class Response:
            status = 200

            def __init__(self, body=b"# TYPE worker counter\nworker{role=\"live\",strategy=\"live-momentum\"} 1\n# TYPE app_strategy_worker_insight_route_duration_seconds histogram\napp_strategy_worker_insight_route_duration_seconds_bucket{role=\"scout\",strategy=\"live-momentum\",le=\"0.005\"} 1\n"):
                self.body = body

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return None

            def read(self):
                return self.body

        requested = []

        def open_url(url, timeout):
            requested.append((url, timeout))
            return Response()

        output = []
        with patch.object(exporter, "urlopen", side_effect=open_url):
            exporter.strategy_worker_metrics(output)

        self.assertEqual(len(exporter.STRATEGY_WORKER_STRATEGIES) * 2, len(requested))
        self.assertTrue(all(timeout == 8 for _, timeout in requested))
        self.assertTrue(any("strategy-live-momentum:10202/metrics" in url for url, _ in requested))
        self.assertTrue(any("strategy-scout-live-momentum:10202/metrics" in url for url, _ in requested))
        self.assertTrue(any("app_strategy_worker_up" in line and 'role="scout"' in line for line in output))
        self.assertTrue(any("app_strategy_worker_memory_limit_bytes" in line and 'role="live"' in line for line in output))
        self.assertTrue(any("app_strategy_worker_insight_route_duration_seconds_bucket" in line for line in output))
        self.assertFalse(any("symbol=" in line for line in output))

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
