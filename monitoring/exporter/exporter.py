import asyncio
import concurrent.futures
import json
import os
import re
import threading
import time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.error import URLError
from urllib.request import urlopen


SERVICES = {
    "feed_handlers": (os.getenv("FEED_HANDLERS_URL", "http://127.0.0.1:3001"), "/health"),
    "binance_feed": (os.getenv("BINANCE_FEED_HEALTH_URL", "http://127.0.0.1:3012"), "/health"),
    "position_management": (os.getenv("POSITION_MANAGEMENT_URL", "http://127.0.0.1:3002"), "/health"),
    "auto_trader": (os.getenv("AUTO_TRADER_URL", "http://127.0.0.1:3005"), "/health"),
    "execution": (os.getenv("EXECUTION_URL", "http://127.0.0.1:3003"), "/health"),
    "insights": (os.getenv("INSIGHTS_URL", "http://127.0.0.1:3004"), "/health"),
    "live_signals": (os.getenv("LIVE_SIGNALS_URL", "http://127.0.0.1:3007"), "/health"),
    "news_sentiment": (os.getenv("NEWS_SENTIMENT_URL", "http://127.0.0.1:3006"), "/health"),
    "learning": (os.getenv("LEARNING_SERVICE_URL", "http://127.0.0.1:3010"), "/health"),
    "scouts": (os.getenv("SCOUTS_SERVICE_URL", "http://127.0.0.1:3011"), "/health"),
}
RSOCKET_SERVICES = {
    "position_management": os.getenv(
        "POSITION_MANAGEMENT_RSOCKET_URL", "ws://127.0.0.1:10005"
    ),
    "auto_trader": os.getenv("AUTO_TRADER_RSOCKET_URL", "ws://127.0.0.1:10008"),
    "learning": os.getenv("LEARNING_RSOCKET_URL", "ws://127.0.0.1:10010"),
}

_RSOCKET_LOOP = None
_RSOCKET_LOOP_THREAD = None
_RSOCKET_LOOP_LOCK = threading.Lock()
_RSOCKET_CLIENTS = {}
_RSOCKET_CLIENT_LOCKS = {}


def get_json(base, path):
    with urlopen(base + path, timeout=3) as response:
        return json.load(response)


async def _rsocket_request(endpoint, request):
    from rsocket.payload import Payload
    from rsocket.exceptions import RSocketNoAvailableTransport, RSocketTransportClosed, RSocketTransportError

    lock = _RSOCKET_CLIENT_LOCKS.setdefault(endpoint, asyncio.Lock())
    async with lock:
        context, client = _RSOCKET_CLIENTS.get(endpoint, (None, None))
        if client is None:
            context = _websocket_client_context(endpoint)
            client = await context.__aenter__()
            _RSOCKET_CLIENTS[endpoint] = (context, client)
    try:
        response = await asyncio.wait_for(
            client.request_response(
                Payload(data=json.dumps(request, separators=(",", ":")).encode("utf-8"))
            ),
            timeout=3,
        )
    except (RSocketNoAvailableTransport, RSocketTransportClosed, RSocketTransportError, asyncio.TimeoutError):
        await _discard_rsocket_client(endpoint, context, client)
        raise
    if not response or not response.data:
        raise ValueError("RSocket service returned an empty response")
    return json.loads(response.data.decode("utf-8"))


def _websocket_client_context(endpoint):
    from rsocket.transports.aiohttp_websocket import websocket_client

    return websocket_client(endpoint)


async def _discard_rsocket_client(endpoint, context, client):
    cached = _RSOCKET_CLIENTS.get(endpoint)
    if cached is None or cached[1] is not client:
        return
    _RSOCKET_CLIENTS.pop(endpoint, None)
    try:
        transport = await client._current_transport()
        websocket = transport._ws
        connection = websocket._response.connection
        if connection is not None and connection.transport is not None:
            connection.transport.abort()
    except Exception:
        pass
    try:
        await context.__aexit__(None, None, None)
    except Exception:
        pass


def _rsocket_loop():
    global _RSOCKET_LOOP, _RSOCKET_LOOP_THREAD
    with _RSOCKET_LOOP_LOCK:
        if _RSOCKET_LOOP is None:
            loop = asyncio.new_event_loop()

            def run_loop():
                asyncio.set_event_loop(loop)
                loop.run_forever()

            thread = threading.Thread(target=run_loop, name="exporter-rsocket", daemon=True)
            thread.start()
            _RSOCKET_LOOP = loop
            _RSOCKET_LOOP_THREAD = thread
        return _RSOCKET_LOOP


def _run_rsocket_request(endpoint, request):
    future = asyncio.run_coroutine_threadsafe(_rsocket_request(endpoint, request), _rsocket_loop())
    try:
        return future.result(timeout=3.5)
    except concurrent.futures.TimeoutError as error:
        future.cancel()
        raise TimeoutError(f"RSocket request to {endpoint} timed out") from error


def rsocket_get(endpoint, path):
    try:
        result = _run_rsocket_request(endpoint, {"method": "GET", "path": path})
    except Exception as error:
        raise OSError(f"RSocket request to {endpoint} failed: {error}") from error
    status = result.get("status")
    if not isinstance(status, int) or status < 200 or status >= 300:
        raise ValueError(f"RSocket request failed with status {status}")
    return result.get("body")


def learning_get(endpoint, path):
    request = {
        "operation": "worker_route",
        "request": {
            "request_id": f"monitoring-exporter-{time.time_ns()}",
            "strategy": None,
            "method": "get",
            "path": path,
            "payload": None,
        },
    }
    try:
        result = _run_rsocket_request(endpoint, request)
    except Exception as error:
        raise OSError(f"Learning RSocket request to {endpoint} failed: {error}") from error
    route_result = result.get("worker_route", {}).get("response", {})
    status = route_result.get("status")
    if not isinstance(status, int) or status < 200 or status >= 300:
        raise ValueError(f"Learning route failed with status {status}")
    return route_result.get("body")


METRIC_HEADERS = set()
LAST_EXPORTED_ORDER_STAGE_TIMESTAMP = {}


def sample(name, help_text, value, labels=None, kind="gauge", timestamp_ms=None):
    labels_text = ""
    if labels:
        labels_text = "{" + ",".join(f'{key}="{str(val).replace(chr(92), chr(92) * 2).replace(chr(34), chr(92) + chr(34))}"' for key, val in labels.items()) + "}"
    if name not in METRIC_HEADERS:
        header = "# HELP " + name + " " + help_text + chr(10) + "# TYPE " + name + " " + kind + chr(10)
        METRIC_HEADERS.add(name)
    else:
        header = ""
    timestamp_text = "" if timestamp_ms is None else " " + str(timestamp_ms)
    return header + name + labels_text + " " + str(value) + timestamp_text + chr(10)


def insights_maintenance_metrics(output):
    database_path = Path(os.getenv("SQLITE_DB_PATH", "/workspace/src/rust/insights/signals.sqlite"))
    status_path = Path(os.getenv("INSIGHTS_MAINTENANCE_STATUS_FILE") or database_path.with_name("maintenance_status.json"))
    try:
        with status_path.open(encoding="utf-8") as status_file:
            status = json.load(status_file)
    except (OSError, ValueError) as error:
        status = {}
        status_error = f"Maintenance status unavailable: {error}"
    else:
        status_error = status.get("last_error")

    heartbeat_ms = status.get("heartbeat_at_ms")
    alive = isinstance(heartbeat_ms, (int, float)) and 0 <= (int(time.time() * 1000) - heartbeat_ms) <= 90_000
    output.append(sample("app_service_up", "Whether an application service is available.", int(alive), {"service": "insights_maintenance"}))
    metrics = (
        ("last_attempt_at_ms", "app_insights_maintenance_last_attempt_timestamp_seconds", "Unix timestamp of the most recent Insights maintenance attempt."),
        ("last_success_at_ms", "app_insights_maintenance_last_success_timestamp_seconds", "Unix timestamp of the most recent successful Insights maintenance run."),
    )
    for field, name, help_text in metrics:
        value = status.get(field)
        if isinstance(value, (int, float)) and value > 0:
            output.append(sample(name, help_text, value / 1000))

    last_run_success = status.get("last_run_success")
    output.append(sample("app_insights_maintenance_last_run_success", "Whether the most recent Insights maintenance run succeeded; -1 means no run has completed.", int(last_run_success) if isinstance(last_run_success, bool) else -1))
    last_attempt = status.get("last_attempt_at_ms")
    output.append(sample("app_insights_maintenance_has_run", "Whether Insights maintenance has started at least one run.", int(isinstance(last_attempt, (int, float)) and last_attempt > 0)))
    for field, name, help_text in (
        ("last_run_archived_records", "app_insights_maintenance_last_run_archived_records", "Signal records included in training archives during the most recent maintenance run."),
        ("last_run_purged_signal_records", "app_insights_maintenance_last_run_purged_signal_records", "Raw signal records purged during the most recent maintenance run."),
        ("last_run_compacted_audit_records", "app_insights_maintenance_last_run_compacted_audit_records", "Audit records summarized during the most recent maintenance run."),
        ("last_run_purged_audit_records", "app_insights_maintenance_last_run_purged_audit_records", "Detailed audit records purged during the most recent maintenance run."),
        ("last_run_expired_training_archives", "app_insights_maintenance_last_run_expired_training_archives", "Training archives expired during the most recent maintenance run."),
        ("consecutive_failures", "app_insights_maintenance_consecutive_failures", "Consecutive failed Insights maintenance runs."),
    ):
        value = status.get(field)
        if isinstance(value, (int, float)):
            output.append(sample(name, help_text, value))
    error = str(status_error or "").replace("\n", " ").replace("\r", " ")[:300]
    output.append(sample("app_insights_maintenance_last_error_present", "Whether Insights maintenance has a current error.", int(bool(error))))
    if error:
        output.append(sample("app_insights_maintenance_last_error_info", "Current Insights maintenance error details.", 1, {"error": error}))


def insights_signal_latency_metrics(output):
    base = SERVICES["live_signals"][0]
    try:
        latency = get_json(base, "/internal/latency")
    except (OSError, URLError, ValueError, TypeError):
        latency = {}

    for stage, summary in latency.items():
        if not isinstance(summary, dict):
            continue
        for stat in ("p50_us", "p95_us"):
            value = summary.get(stat)
            if isinstance(value, (int, float)):
                output.append(sample(
                    "app_live_signals_latency_microseconds",
                    "Approximate Live Signals pipeline latency percentile in microseconds.",
                    value,
                    {"stage": stage, "stat": stat.removesuffix("_us")},
                ))
        count = summary.get("count")
        if isinstance(count, (int, float)):
            output.append(sample(
                "app_live_signals_latency_observations_total",
                "Cumulative Live Signals latency observations.",
                count,
                {"stage": stage},
                kind="counter",
            ))

    try:
        research_latency = get_json(SERVICES["insights"][0], "/internal/latency")
    except (OSError, URLError, ValueError, TypeError):
        research_latency = {}
    dropped = research_latency.get("dropped_research_records")
    if isinstance(dropped, (int, float)):
        output.append(sample(
            "app_insights_research_records_dropped_total",
            "Cumulative number of research records dropped because the bounded Insights writer queue was full.",
            dropped,
            kind="counter",
        ))


def metrics():
    METRIC_HEADERS.clear()
    output = []
    db_paths = os.getenv("SQLITE_DB_PATHS", os.getenv("SQLITE_DB_PATH", "")).split(os.pathsep)
    for configured_path in db_paths:
        db_path = Path(configured_path)
        if db_path.is_file():
            output.append(sample("app_sqlite_database_size_bytes", "SQLite database file size in bytes.", db_path.stat().st_size, {"database": db_path.name}))
    store_paths = os.getenv("PERSISTENT_STORE_PATHS", "").split(os.pathsep)
    for configured_path in store_paths:
        store_path = Path(configured_path)
        if store_path.is_file():
            output.append(sample("app_persistent_store_size_bytes", "Persistent application store file size in bytes.", store_path.stat().st_size, {"store": store_path.name}))
    service_up = {}
    for name, (base, health_path) in SERVICES.items():
        try:
            with urlopen(base + health_path, timeout=2) as response:
                up = 1 if response.status == 200 else 0
        except (OSError, URLError):
            up = 0
        service_up[name] = up
        output.append(sample("app_service_up", "Whether an application service health endpoint is reachable.", up, {"service": name}))
        try:
            with urlopen(base + "/process-cpu-seconds", timeout=2) as response:
                cpu_seconds = float(response.read().decode().strip())
            output.append(sample("app_service_process_cpu_seconds_total", "Cumulative CPU time consumed by an application service process in seconds.", cpu_seconds, {"service": name}, kind="counter"))
            with urlopen(base + "/process-memory-bytes", timeout=2) as response:
                memory_bytes = int(response.read().decode().strip())
            output.append(sample("app_service_process_resident_memory_bytes", "Current resident memory (RSS) used by an application service process in bytes.", memory_bytes, {"service": name}))
        except (OSError, URLError, ValueError):
            pass

    insights_maintenance_metrics(output)
    insights_signal_latency_metrics(output)

    try:
        scouts = learning_get(
            RSOCKET_SERVICES["learning"], "/outcomes?source=all&page_size=1"
        )
        summary = scouts.get("summary", {}).get("all", {})
        total = summary.get("total")
        open_count = summary.get("open")
        if isinstance(total, (int, float)) and isinstance(open_count, (int, float)):
            output.append(sample("app_scout_instances", "Scout instances in the recent seven-day report, by lifecycle status.", open_count, {"status": "open"}))
            output.append(sample("app_scout_instances", "Scout instances in the recent seven-day report, by lifecycle status.", max(0, total - open_count), {"status": "closed"}))
    except (OSError, URLError, ValueError, TypeError, AttributeError):
        pass

    try:
        persistence = rsocket_get(
            RSOCKET_SERVICES["position_management"], "/internal/persistence-timing"
        )
        for phase, summary in persistence.items():
            stage = f"position_manager_snapshot_{phase}"
            for stat in ("latest", "average", "p95"):
                value = summary.get(f"{stat}_ms")
                if isinstance(value, (int, float)):
                    output.append(sample("app_autotrader_stage_latency_ms", "Milliseconds spent serializing and writing a background position-ledger snapshot.", value, {"stage": stage, "stat": stat}))
            if isinstance(summary.get("samples"), (int, float)):
                output.append(sample("app_autotrader_stage_latency_samples", "Recent background position-ledger snapshot measurements.", summary["samples"], {"stage": stage}))
    except (OSError, URLError, ValueError, TypeError):
        pass
    try:
        account = rsocket_get(RSOCKET_SERVICES["position_management"], "/positions/account")
        for key, metric, help_text in [
            ("equity", "app_account_equity_usd", "Current account equity in USD."),
            ("cash", "app_account_cash_usd", "Current account cash in USD."),
            ("realized_pnl", "app_account_realized_pnl_usd", "Cumulative account realized P and L in USD."),
            ("unrealized_pnl", "app_account_unrealized_pnl_usd", "Current account unrealized P and L in USD."),
            ("fees_paid", "app_account_fees_paid_usd", "Cumulative account fees in USD."),
        ]:
            if isinstance(account.get(key), (int, float)):
                output.append(sample(metric, help_text, account[key]))
    except (OSError, URLError, ValueError):
        pass
    try:
        positions = rsocket_get(RSOCKET_SERVICES["position_management"], "/positions")
        if isinstance(positions.get("trade_count"), (int, float)):
            output.append(sample("app_trade_count", "Cumulative number of persisted position ledger fill records.", positions["trade_count"], kind="counter"))
        open_positions = sum(1 for position in positions.get("positions", []) if abs(position.get("quantity", 0)) > 1e-12)
        output.append(sample("app_open_positions", "Number of currently open account positions.", open_positions))
    except (OSError, URLError, ValueError, TypeError):
        pass
    try:
        status = rsocket_get(RSOCKET_SERVICES["auto_trader"], "/positions/auto-trader")
        enabled = bool(status.get("enabled"))
        output.append(sample("app_autotrader_enabled", "Whether AutoTrader is enabled according to its status endpoint.", int(enabled)))
        latest_diagnostic = status.get("latest_signal_diagnostic", "")
        if isinstance(latest_diagnostic, str):
            stale_match = re.search(r"filtered:\s*(\d+)\s+stale\b", latest_diagnostic)
            if stale_match:
                output.append(sample(
                    "app_autotrader_latest_stale_signals",
                    "Number of signals rejected as stale in the latest AutoTrader decision.",
                    int(stale_match.group(1)),
                ))
        stale_candidates_total = status.get("stale_signal_candidates_total")
        if isinstance(stale_candidates_total, (int, float)):
            output.append(sample(
                "app_autotrader_stale_signal_candidates_total",
                "Cumulative live and strategy signal candidates rejected for stale timestamps or stale flags.",
                stale_candidates_total,
                kind="counter",
            ))
        stale_signal_cycles_total = status.get("stale_signal_cycles_total")
        if isinstance(stale_signal_cycles_total, (int, float)):
            output.append(sample(
                "app_autotrader_stale_signal_cycles_total",
                "Cumulative decision cycles that contained at least one stale signal candidate.",
                stale_signal_cycles_total,
                kind="counter",
            ))
        for key, metric, help_text in [
            ("used_notional_today", "app_autotrader_used_notional_today_usd", "Autotrader notional used today in USD."),
            ("daily_notional_limit", "app_autotrader_daily_notional_limit_usd", "Autotrader daily notional limit in USD."),
        ]:
            if isinstance(status.get(key), (int, float)):
                output.append(sample(metric, help_text, status[key]))
    except (OSError, URLError, ValueError, TypeError):
        # A failed status request is unknown, not disabled. Emitting zero here
        # makes Grafana report DISABLED while the UI can still show its last
        # successfully fetched enabled state.
        pass
    try:
        activity = rsocket_get(
            RSOCKET_SERVICES["auto_trader"], "/positions/auto-trader/activity"
        )
        value = activity.get("insight_processing_latest_ms")
        if isinstance(value, (int, float)):
            output.append(sample("app_autotrader_insight_processing_duration_ms", "Latest milliseconds spent processing one pushed insight through an AutoTrader cycle.", value, {"stat": "latest"}))
            output.append(sample("app_autotrader_stage_latency_ms", "Latest milliseconds spent in an AutoTrader cycle or recorded order pipeline stage.", value, {"stage": "cycle_total", "stat": "latest"}))
        if isinstance(activity.get("insight_processing_samples"), (int, float)):
            output.append(sample("app_autotrader_insight_processing_samples", "Number of recent pushed insight processing cycles included in the duration summary.", activity["insight_processing_samples"]))
            output.append(sample("app_autotrader_stage_latency_samples", "Number of recent insight cycles included in the cycle total duration summary.", activity["insight_processing_samples"], {"stage": "cycle_total"}))
        settings_persistence = activity.get("settings_persistence_ms", {})
        if isinstance(settings_persistence, dict):
            for phase, summary in settings_persistence.items():
                for stat in ("latest", "average", "p95"):
                    value = summary.get(f"{stat}_ms")
                    if isinstance(value, (int, float)):
                        output.append(sample("app_autotrader_stage_latency_ms", "Milliseconds spent serializing or writing a background AutoTrader state snapshot.", value, {"stage": f"settings_{phase}", "stat": stat}))
                if isinstance(summary.get("samples"), (int, float)):
                    output.append(sample("app_autotrader_stage_latency_samples", "Recent AutoTrader settings snapshot measurements.", summary["samples"], {"stage": f"settings_{phase}"}))
        for phase, field in (("wait", "cycle_lock_wait_ms"), ("hold", "cycle_lock_hold_ms")):
            worker_summaries = activity.get(field, {})
            if not isinstance(worker_summaries, dict):
                continue
            for worker, summary in worker_summaries.items():
                if not isinstance(summary, dict):
                    continue
                for stat in ("latest", "average", "p95"):
                    value = summary.get(f"{stat}_ms")
                    if isinstance(value, (int, float)):
                        output.append(sample("app_autotrader_cycle_lock_duration_ms", "Milliseconds waiting for or holding the shared AutoTrader cycle lock; summary stats cover recent observations.", value, {"phase": phase, "worker": worker, "stat": stat}))
                if isinstance(summary.get("samples"), (int, float)):
                    output.append(sample("app_autotrader_cycle_lock_duration_samples", "Recent lock duration observations for an AutoTrader worker.", summary["samples"], {"phase": phase, "worker": worker}))
        cycle_lock_scopes = activity.get("cycle_lock_scope_ms", {})
        if not isinstance(cycle_lock_scopes, dict):
            cycle_lock_scopes = {}
        for scope, summary in cycle_lock_scopes.items():
            if not isinstance(summary, dict):
                continue
            for stat in ("latest", "average", "p95"):
                value = summary.get(f"{stat}_ms")
                if isinstance(value, (int, float)):
                    output.append(sample("app_autotrader_cycle_lock_scope_duration_ms", "Milliseconds spent in measured sections while holding the AutoTrader cycle lock.", value, {"scope": scope, "stat": stat}))
            if isinstance(summary.get("samples"), (int, float)):
                output.append(sample("app_autotrader_cycle_lock_scope_duration_samples", "Recent observations for a measured AutoTrader cycle-lock section.", summary["samples"], {"scope": scope}))
        for key, metric, help_text in [
            ("realized_pnl", "app_autotrader_realized_pnl_usd", "Autotrader cumulative realized P and L in USD."),
            ("unrealized_pnl", "app_autotrader_unrealized_pnl_usd", "Autotrader unrealized P and L in USD."),
            ("learning_samples", "app_autotrader_learning_samples", "Autotrader closed trade learning samples."),
            ("learning_win_rate", "app_autotrader_learning_win_rate", "Autotrader closed trade win rate from zero to one."),
            ("closed_trade_wins", "app_autotrader_closed_trade_wins", "Number of closed winning autotrader trades."),
            ("closed_trade_losses", "app_autotrader_closed_trade_losses", "Number of closed losing autotrader trades."),
            ("closed_trade_flat", "app_autotrader_closed_trade_flat", "Number of closed near-flat autotrader trades."),
        ]:
            if isinstance(activity.get(key), (int, float)):
                output.append(sample(metric, help_text, activity[key]))
        for item in activity.get("pnl_breakdown", []):
            strategy = item.get("strategy", "legacy-unknown")
            symbol = item.get("symbol", "")
            quote = symbol.rsplit("-", 1)[-1] if "-" in symbol else "unknown"
            value = item.get("realized_pnl")
            if isinstance(value, (int, float)):
                output.append(sample("app_autotrader_strategy_realized_pnl", "Recent realized P and L grouped by AutoTrader strategy and quote currency.", value, {"strategy": strategy, "quote": quote}))
            outcomes = item.get("outcomes")
            if isinstance(outcomes, (int, float)):
                output.append(sample("app_autotrader_strategy_outcomes", "Number of realized trade outcomes grouped by AutoTrader strategy and quote currency.", outcomes, {"strategy": strategy, "quote": quote}))
        orders = activity.get("orders", [])
        # Use the position manager's live order book, not AutoTrader's retained
        # activity log. Historical statuses such as "armed" can remain after an
        # exchange order is gone.
        live_orders = rsocket_get(
            RSOCKET_SERVICES["position_management"], "/positions/orders"
        )
        resting_orders = [order for order in live_orders.get("orders", []) if order.get("source") == "auto"]
        barrier_orders = live_orders.get("barrier_orders", [])
        active_barriers = [order for order in barrier_orders if order.get("source") == "auto" and order.get("status") in ("armed", "triggering")]
        live_count = len(resting_orders) + len(active_barriers)
        output.append(sample("app_autotrader_orders", "Number of currently open resting and armed AutoTrader orders.", live_count))
        output.append(sample("app_autotrader_orders_by_status", "Currently open AutoTrader orders by live order type.", len(resting_orders), {"status": "resting"}))
        output.append(sample("app_autotrader_orders_by_status", "Currently open AutoTrader orders by live order type.", sum(1 for order in active_barriers if order.get("status") == "armed"), {"status": "armed"}))
        output.append(sample("app_autotrader_orders_by_status", "Currently open AutoTrader orders by live order type.", sum(1 for order in active_barriers if order.get("status") == "triggering"), {"status": "triggering"}))
        # A signal's source timestamp can precede its selection by many seconds
        # (and is already reported as source_signal_age below). Do not label
        # that age as order-creation processing latency. Use the measured cycle
        # duration recorded immediately before the order is persisted.
        order_creation = [
            (int(order.get("timestamp", 0)), order["stage_timings_ms"]["cycle_to_order_record"])
            for order in orders
            if order.get("purpose") == "signal"
            and isinstance(order.get("stage_timings_ms"), dict)
            and isinstance(order["stage_timings_ms"].get("cycle_to_order_record"), (int, float))
            and order["stage_timings_ms"]["cycle_to_order_record"] >= 0
        ]
        if order_creation:
            _, latest_latency = max(order_creation, key=lambda item: item[0])
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Latest measured AutoTrader cycle processing milliseconds from cycle start to signal order creation.", latest_latency, {"stat": "latest"}))
            output.append(sample("app_autotrader_tick_to_order_samples", "Number of retained signal orders with a measured cycle-to-order duration.", len(order_creation)))
        stage_values = {}
        for order in orders:
            for stage, latency in order.get("stage_timings_ms", {}).items():
                if isinstance(latency, (int, float)) and latency >= 0:
                    stage_values.setdefault(stage, []).append((int(order.get("timestamp", 0)), latency))
        for stage, samples in stage_values.items():
            # Emit each order-stage observation once. Re-exporting the latest
            # value on every scrape makes event timings look like repeated data.
            metric_stage = "source_signal_age" if stage == "signal_age_at_order_record" else stage
            description = "Latest milliseconds per AutoTrader stage; source_signal_age is signal age, not processing time."
            last_timestamp = LAST_EXPORTED_ORDER_STAGE_TIMESTAMP.get(metric_stage, -1)
            new_samples = sorted((timestamp, value) for timestamp, value in samples if timestamp > last_timestamp)
            for event_timestamp, value in new_samples:
                output.append(sample("app_autotrader_stage_latency_ms", description, value, {"stage": metric_stage, "stat": "latest"}))
                LAST_EXPORTED_ORDER_STAGE_TIMESTAMP[metric_stage] = event_timestamp

    except (OSError, URLError, ValueError, TypeError):
        pass
    try:
        counts = get_json(SERVICES["feed_handlers"][0], "/internal/message-counts")
        for feed in ("trades", "books", "tickers"):
            value = counts.get(feed)
            if isinstance(value, (int, float)):
                output.append(sample("app_feed_messages_total", "Cumulative accepted market feed updates by channel.", value, {"channel": feed}, kind="counter"))
    except (OSError, URLError, ValueError, TypeError):
        pass
    try:
        for queue in get_json(SERVICES["feed_handlers"][0], "/internal/queue-metrics"):
            labels = {"queue": queue.get("queue", "unknown")}
            output.append(sample("app_feed_queue_size", "Current number of retained messages in a feed handler queue.", queue.get("depth", 0), labels))
            output.append(sample("app_feed_queue_dropped_messages_total", "Cumulative messages skipped by lagging feed handler queue receivers.", queue.get("dropped", 0), labels, kind="counter"))
    except (OSError, URLError, ValueError, TypeError):
        pass
    return "".join(output).encode()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/metrics":
            body = metrics()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == "/health":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
        else:
            self.send_error(404)

    def log_message(self, *_):
        pass


def main():
    HTTPServer(("0.0.0.0", 9108), Handler).serve_forever()


if __name__ == "__main__":
    main()
