import json
import os
from pathlib import Path
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.error import URLError
from urllib.request import urlopen

SERVICES = {
    "feed_handlers": (os.getenv("FEED_HANDLERS_URL", "http://127.0.0.1:3001"), "/health"),
    "position_management": (os.getenv("POSITION_MANAGEMENT_URL", "http://127.0.0.1:3002"), "/health"),
    "auto_trader": (os.getenv("AUTO_TRADER_URL", "http://127.0.0.1:3005"), "/health"),
    "execution": (os.getenv("EXECUTION_URL", "http://127.0.0.1:3003"), "/health"),
    "insights": (os.getenv("INSIGHTS_URL", "http://127.0.0.1:3004"), "/health"),
    "news_sentiment": (os.getenv("NEWS_SENTIMENT_URL", "http://127.0.0.1:3006"), "/health"),
}


def get_json(base, path):
    with urlopen(base + path, timeout=3) as response:
        return json.load(response)


METRIC_HEADERS = set()


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


def metrics():
    METRIC_HEADERS.clear()
    output = []
    db_path = Path(os.getenv("SQLITE_DB_PATH", ""))
    if db_path.is_file():
        output.append(sample("app_sqlite_database_size_bytes", "SQLite database file size in bytes.", db_path.stat().st_size, {"database": db_path.name}))
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
        except (OSError, URLError, ValueError):
            pass

    base = SERVICES["position_management"][0]
    try:
        account = get_json(base, "/positions/account")
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
        positions = get_json(base, "/positions")
        if isinstance(positions.get("trade_count"), (int, float)):
            output.append(sample("app_trade_count", "Cumulative number of persisted position ledger fill records.", positions["trade_count"], kind="counter"))
        open_positions = sum(1 for position in positions.get("positions", []) if abs(position.get("quantity", 0)) > 1e-12)
        output.append(sample("app_open_positions", "Number of currently open account positions.", open_positions))
    except (OSError, URLError, ValueError, TypeError):
        pass
    auto_trader_base = SERVICES["auto_trader"][0]
    try:
        status = get_json(auto_trader_base, "/positions/auto-trader")
        enabled = bool(status.get("enabled"))
        output.append(sample("app_autotrader_enabled", "Whether AutoTrader is enabled according to its status endpoint.", int(enabled)))
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
        activity = get_json(auto_trader_base, "/positions/auto-trader/activity")
        for key, stat in [("insight_processing_latest_ms", "latest"), ("insight_processing_average_ms", "average"), ("insight_processing_p95_ms", "p95")]:
            value = activity.get(key)
            if isinstance(value, (int, float)):
                output.append(sample("app_autotrader_insight_processing_duration_ms", "Milliseconds spent processing one pushed insight through an AutoTrader cycle.", value, {"stat": stat}))
                output.append(sample("app_autotrader_stage_latency_ms", "Milliseconds spent in an AutoTrader cycle or recorded order pipeline stage.", value, {"stage": "cycle_total", "stat": stat}))
        if isinstance(activity.get("insight_processing_samples"), (int, float)):
            output.append(sample("app_autotrader_insight_processing_samples", "Number of recent pushed insight processing cycles included in the duration summary.", activity["insight_processing_samples"]))
            output.append(sample("app_autotrader_stage_latency_samples", "Number of recent insight cycles included in the cycle total duration summary.", activity["insight_processing_samples"], {"stage": "cycle_total"}))
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
                        output.append(sample("app_autotrader_cycle_lock_duration_ms", "Milliseconds waiting for or holding the shared AutoTrader cycle lock.", value, {"phase": phase, "worker": worker, "stat": stat}))
                if isinstance(summary.get("samples"), (int, float)):
                    output.append(sample("app_autotrader_cycle_lock_duration_samples", "Recent lock duration observations for an AutoTrader worker.", summary["samples"], {"phase": phase, "worker": worker}))
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
        output.append(sample("app_autotrader_orders", "Number of recorded autotrader orders.", len(orders)))
        for status_name in sorted({order.get("status", "unknown") for order in orders}):
            output.append(sample("app_autotrader_orders_by_status", "Recorded autotrader orders by status.", sum(1 for order in orders if order.get("status", "unknown") == status_name), {"status": status_name}))
        tick_to_order = []
        for order in orders:
            if order.get("purpose") != "signal":
                continue
            decision_id = order.get("decision_id")
            try:
                signal_timestamp = int(decision_id.split("|", 1)[0])
                order_timestamp = int(order.get("timestamp", 0))
                latency = order_timestamp - signal_timestamp
                if signal_timestamp > 0 and latency >= 0:
                    tick_to_order.append((order_timestamp, latency))
            except (AttributeError, TypeError, ValueError):
                continue
        if tick_to_order:
            latencies = sorted(latency for _, latency in tick_to_order)
            p95_index = max(0, (95 * len(latencies) + 99) // 100 - 1)
            event_timestamp = max(timestamp for timestamp, _ in tick_to_order)
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Milliseconds from signal tick timestamp to autotrader order creation.", latencies[-1], {"stat": "latest"}, timestamp_ms=event_timestamp))
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Milliseconds from signal tick timestamp to autotrader order creation.", sum(latencies) / len(latencies), {"stat": "average"}, timestamp_ms=event_timestamp))
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Milliseconds from signal tick timestamp to autotrader order creation.", latencies[p95_index], {"stat": "p95"}, timestamp_ms=event_timestamp))
            output.append(sample("app_autotrader_tick_to_order_samples", "Number of signal orders used in the tick-to-order latency summary.", len(latencies)))
        stage_values = {}
        for order in orders:
            for stage, latency in order.get("stage_timings_ms", {}).items():
                if isinstance(latency, (int, float)) and latency >= 0:
                    stage_values.setdefault(stage, []).append((int(order.get("timestamp", 0)), latency))
        for stage, samples in stage_values.items():
            values = sorted(value for _, value in samples)
            p95_index = max(0, (95 * len(values) + 99) // 100 - 1)
            event_timestamp = max(timestamp for timestamp, _ in samples)
            output.append(sample("app_autotrader_stage_latency_ms", "Milliseconds spent in a measured AutoTrader pipeline stage.", values[-1], {"stage": stage, "stat": "latest"}, timestamp_ms=event_timestamp))
            output.append(sample("app_autotrader_stage_latency_ms", "Milliseconds spent in a measured AutoTrader pipeline stage.", sum(values) / len(values), {"stage": stage, "stat": "average"}, timestamp_ms=event_timestamp))
            output.append(sample("app_autotrader_stage_latency_ms", "Milliseconds spent in a measured AutoTrader pipeline stage.", values[p95_index], {"stage": stage, "stat": "p95"}, timestamp_ms=event_timestamp))
            output.append(sample("app_autotrader_stage_latency_samples", "Number of recent AutoTrader orders with a measurement for this pipeline stage.", len(values), {"stage": stage}, timestamp_ms=event_timestamp))
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


HTTPServer(("0.0.0.0", 9108), Handler).serve_forever()
