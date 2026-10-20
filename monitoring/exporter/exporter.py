import json
import os
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.error import URLError
from urllib.request import urlopen

SERVICES = {
    "feed_handlers": (os.getenv("FEED_HANDLERS_URL", "http://127.0.0.1:3001"), "/health"),
    "position_management": (os.getenv("POSITION_MANAGEMENT_URL", "http://127.0.0.1:3002"), "/health"),
    "execution": (os.getenv("EXECUTION_URL", "http://127.0.0.1:3003"), "/health"),
    "insights": (os.getenv("INSIGHTS_URL", "http://127.0.0.1:3004"), "/health"),
}


def get_json(base, path):
    with urlopen(base + path, timeout=3) as response:
        return json.load(response)


METRIC_HEADERS = set()


def sample(name, help_text, value, labels=None, kind="gauge"):
    labels_text = ""
    if labels:
        labels_text = "{" + ",".join(f'{key}="{str(val).replace(chr(92), chr(92) * 2).replace(chr(34), chr(92) + chr(34))}"' for key, val in labels.items()) + "}"
    if name not in METRIC_HEADERS:
        header = "# HELP " + name + " " + help_text + chr(10) + "# TYPE " + name + " " + kind + chr(10)
        METRIC_HEADERS.add(name)
    else:
        header = ""
    return header + name + labels_text + " " + str(value) + chr(10)


def metrics():
    METRIC_HEADERS.clear()
    output = []
    for name, (base, health_path) in SERVICES.items():
        try:
            with urlopen(base + health_path, timeout=2) as response:
                up = 1 if response.status == 200 else 0
        except (OSError, URLError):
            up = 0
        output.append(sample("app_service_up", "Whether an application service health endpoint is reachable.", up, {"service": name}))

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
            output.append(sample("app_trade_count", "Number of persisted position ledger trades.", positions["trade_count"], kind="counter"))
        open_positions = sum(1 for position in positions.get("positions", []) if abs(position.get("quantity", 0)) > 1e-12)
        output.append(sample("app_open_positions", "Number of currently open account positions.", open_positions))
    except (OSError, URLError, ValueError, TypeError):
        pass
    try:
        status = get_json(base, "/positions/auto-trader")
        output.append(sample("app_autotrader_enabled", "Whether the autotrader is enabled.", int(bool(status.get("enabled")))))
        for key, metric, help_text in [
            ("used_notional_today", "app_autotrader_used_notional_today_usd", "Autotrader notional used today in USD."),
            ("daily_notional_limit", "app_autotrader_daily_notional_limit_usd", "Autotrader daily notional limit in USD."),
        ]:
            if isinstance(status.get(key), (int, float)):
                output.append(sample(metric, help_text, status[key]))
    except (OSError, URLError, ValueError):
        pass
    try:
        activity = get_json(base, "/positions/auto-trader/activity")
        for key, metric, help_text in [
            ("realized_pnl", "app_autotrader_realized_pnl_usd", "Autotrader cumulative realized P and L in USD."),
            ("unrealized_pnl", "app_autotrader_unrealized_pnl_usd", "Autotrader unrealized P and L in USD."),
            ("learning_samples", "app_autotrader_learning_samples", "Autotrader closed trade learning samples."),
            ("learning_win_rate", "app_autotrader_learning_win_rate", "Autotrader closed trade win rate from zero to one."),
            ("closed_trade_wins", "app_autotrader_closed_trade_wins", "Number of closed winning autotrader trades."),
            ("closed_trade_losses", "app_autotrader_closed_trade_losses", "Number of closed losing autotrader trades."),
        ]:
            if isinstance(activity.get(key), (int, float)):
                output.append(sample(metric, help_text, activity[key]))
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
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Milliseconds from signal tick timestamp to autotrader order creation.", latencies[-1], {"stat": "latest"}))
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Milliseconds from signal tick timestamp to autotrader order creation.", sum(latencies) / len(latencies), {"stat": "average"}))
            output.append(sample("app_autotrader_tick_to_order_latency_ms", "Milliseconds from signal tick timestamp to autotrader order creation.", latencies[p95_index], {"stat": "p95"}))
            output.append(sample("app_autotrader_tick_to_order_samples", "Number of signal orders used in the tick-to-order latency summary.", len(latencies)))
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
