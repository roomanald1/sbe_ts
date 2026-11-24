import argparse
import csv
import json
import math
import statistics
import tempfile
from pathlib import Path
from decimal import Decimal
from datetime import datetime, timezone

def parser(description):
    p = argparse.ArgumentParser(description=description)
    p.add_argument("--input", type=Path)
    p.add_argument("--output", type=Path)
    p.add_argument("--json", action="store_true")
    p.add_argument("--limit", type=int, default=20)
    p.add_argument("--max-rows", type=int, default=1000000)
    return p

def load_rows(path, max_rows=1000000):
    if path is None:
        raise SystemExit(
            "BLOCKED: supply a verified data export with --input. "
            "Do not invent repository schemas."
        )
    if path.suffix.lower() == ".csv":
        with path.open(newline="") as f:
            rows = []
            for row in csv.DictReader(f):
                rows.append(row)
                if len(rows) > max_rows:
                    raise SystemExit("BLOCKED: row limit exceeded")
    elif path.suffix.lower() == ".jsonl":
        rows = []
        with path.open() as f:
            for line in f:
                if line.strip():
                    rows.append(json.loads(line))
                    if len(rows) > max_rows:
                        raise SystemExit("BLOCKED: row limit exceeded")
    else:
        if path.stat().st_size > 256 * 1024 * 1024:
            raise SystemExit("BLOCKED: JSON exceeds 256 MiB")
        rows = json.loads(path.read_text())
        if isinstance(rows, dict):
            rows = rows.get("rows")
    if not isinstance(rows, list):
        raise SystemExit("Expected a list of records")
    if len(rows) > max_rows:
        raise SystemExit("BLOCKED: row limit exceeded")
    if any(not isinstance(r, dict) for r in rows):
        raise SystemExit("Expected dictionary records")
    return rows

def require(rows, fields):
    for i, row in enumerate(rows):
        missing = [
            f for f in fields
            if f not in row or row[f] is None or row[f] == ""
        ]
        if missing:
            raise SystemExit(
                f"BLOCKED: row {i} missing {', '.join(missing)}"
            )

def number(value):
    v = float(value)
    if not math.isfinite(v):
        raise ValueError("Non-finite numeric value")
    return v

def decimal(value):
    v = Decimal(str(value))
    if not v.is_finite():
        raise ValueError("Non-finite decimal value")
    return v

def percentile(values, p):
    if not values:
        return None
    values = sorted(values)
    i = (len(values) - 1) * p / 100
    lo, hi = math.floor(i), math.ceil(i)
    return values[lo] + (values[hi] - values[lo]) * (i - lo)

def distribution(values):
    if not values:
        return {"count": 0}
    return {
        "count": len(values),
        "mean": statistics.mean(values),
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
    }

def pnl_stats(values):
    wins = [v for v in values if v > 0]
    losses = [v for v in values if v < 0]
    return {
        "count": len(values),
        "wins": len(wins),
        "losses": len(losses),
        "neutral": len(values) - len(wins) - len(losses),
        "win_rate": len(wins) / len(values) if values else None,
        "net_pnl": sum(values),
        "expectancy": statistics.mean(values) if values else None,
        "avg_win": statistics.mean(wins) if wins else None,
        "avg_loss": statistics.mean(losses) if losses else None,
        "profit_factor": sum(wins) / abs(sum(losses)) if losses else None,
    }

def emit(args, summary, details=None):
    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "summary": summary,
        "details": details,
    }
    if args.output:
        path = args.output
        with path.open("x") as f:
            json.dump(payload, f, indent=2, default=str)
    else:
        with tempfile.NamedTemporaryFile(
            mode="w", prefix="sbe-diagnostic-",
            suffix=".json", delete=False
        ) as f:
            json.dump(payload, f, indent=2, default=str)
            path = Path(f.name)

    result = {"summary": summary, "evidence": str(path)}
    print(json.dumps(result, indent=2, default=str))
