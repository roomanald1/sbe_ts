#!/usr/bin/env python3

import json
import re
import time
import urllib.request
import urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

SERVICES = {
    "autotrader": {
        "port": 11005,
        "files": [
            "src/rust/auto_trader/src/auto_trader/api/http_routes.rs",
            "src/rust/auto_trader/src/browser_rsocket.rs",
        ]
    },
    "insights": {
        "port": 11004,
        "files": [
            "src/rust/insights/src/signal_engine/store/browser_rsocket.rs"
        ]
    },
    "learning": {
        "port": 11010,
        "files": []
    },
    "positions": {
        "port": 11002,
        "files": [
            "src/rust/position_management/src/positions/positions_routes.rs"
        ]
    }
}

SAFE_PROBES = {
    "positions": {
        "/positions",
        "/positions/account",
        "/positions/realized-pnl",
        "/positions/fee-summary",
        "/positions/financial-snapshot",
        "/positions/fee-rates",
        "/positions/usd-rates",
        "/positions/barriers",
        "/internal/persistence-timing",
    },
    "insights": {
        "/signals/dashboard",
        "/signals/best",
        "/signals/auto-trader-dashboard",
    },
    "learning": set(),
    "autotrader": set(),
}

MAX_BYTES = 2_000_000

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None

opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}),
    NoRedirect()
)

def shape(value, depth=0):
    if depth >= 4:
        return type(value).__name__

    if isinstance(value, dict):
        return {
            k: shape(v, depth + 1)
            for k, v in list(value.items())[:40]
        }

    if isinstance(value, list):
        return {
            "type": "array",
            "length": len(value),
            "item": shape(value[0], depth+1)
                    if value else None
        }

    return type(value).__name__

def probe(port, route):
    url = f"http://127.0.0.1:{port}{route}"
    start = time.monotonic()

    try:
        req = urllib.request.Request(
            url,
            method="GET",
            headers={"Accept": "application/json"}
        )

        with opener.open(req, timeout=5) as response:
            raw = response.read(MAX_BYTES+1)

            if len(raw) > MAX_BYTES:
                return {
                    "status": "RESPONSE_TOO_LARGE",
                    "http_status": response.status
                }

            try:
                data = json.loads(raw)
            except ValueError:
                return {
                    "status": "NOT_JSON",
                    "http_status": response.status,
                    "content_type": response.headers.get(
                        "Content-Type"
                    )
                }

            return {
                "status": "JSON_OK",
                "http_status": response.status,
                "bytes": len(raw),
                "elapsed_ms": round(
                    (time.monotonic()-start)*1000, 2
                ),
                "response_shape": shape(data)
            }

    except urllib.error.HTTPError as e:
        return {
            "status": "HTTP_ERROR",
            "http_status": e.code
        }

    except Exception as e:
        return {
            "status": "BLOCKED",
            "error": str(e)
        }

patterns = [
    re.compile(
        r'\.route\(\s*"([^"]+)"\s*,\s*'
        r'(?:axum::routing::)?get\s*\('
    ),
    re.compile(r'\(\s*"GET"\s*,\s*"([^"]+)"\s*\)')
]

catalogue = []

for service, config in SERVICES.items():
    print(f"\n=== {service.upper()} ===")
    routes = {}

    for filename in config["files"]:
        path = ROOT / filename

        if not path.exists():
            print("MISSING:", filename)
            continue

        text = path.read_text()

        for pattern in patterns:
            for match in pattern.finditer(text):
                route = match.group(1)

                routes.setdefault(route, []).append({
                    "file": filename,
                    "line": text.count(
                        "\n", 0, match.start()
                    ) + 1
                })

    for route, sources in sorted(routes.items()):
        item = {
            "service": service,
            "port": config["port"],
            "method": "GET",
            "route": route,
            "sources": sources,
            "financially_verified": False
        }

        if route in SAFE_PROBES[service]:
            item.update(probe(config["port"], route))
        else:
            item["status"] = "DISCOVERED_NOT_TESTED"

        catalogue.append(item)

        print(
            f"{item['status']:24} {route}",
            item.get("http_status", ""),
            item.get("bytes", "")
        )

output = HERE / "discovered_routes.json"
output.write_text(json.dumps(catalogue, indent=2))

print("\n=== SUMMARY ===")
print("Discovered:", len(catalogue))
print("HTTP JSON working:", sum(
    r["status"] == "JSON_OK" for r in catalogue
))
print("Not tested:", sum(
    r["status"] == "DISCOVERED_NOT_TESTED"
    for r in catalogue
))
print("Catalogue:", output)
