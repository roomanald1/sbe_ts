#!/usr/bin/env python3
"""Summarize compressed SignalStore partitions without printing raw market data."""
import gzip
import json
import struct
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SIGNALS_FILE = ROOT / "signals.jsonl"
MAGIC = b"SIGBIN01"
TYPE_NAMES = ("snapshot", "outcome", "evaluation", "book", "research", "external")


def read_u64(buf, offset):
    return struct.unpack_from("<Q", buf, offset)[0], offset + 8


def read_string(buf, offset):
    size, offset = read_u64(buf, offset)
    end = offset + size
    return buf[offset:end].decode("utf-8", "replace"), end


def timestamp_text(timestamp_ms):
    return datetime.fromtimestamp(timestamp_ms / 1000, timezone.utc).isoformat()


def summarize(path):
    raw = path.read_bytes()
    data = gzip.decompress(raw) if raw.startswith(b"\x1f\x8b") else raw
    if not data.startswith(MAGIC):
        return {"file_bytes": len(raw), "error": "Not a SignalStore binary log"}

    counts = Counter()
    symbols = Counter()
    directions = Counter()
    timestamp_bounds = defaultdict(lambda: [None, None])
    records = data[MAGIC.__len__():]
    offset = 0
    malformed = 0

    while offset < len(records):
        if len(records) - offset < 4:
            malformed += 1
            break
        size = struct.unpack_from("<I", records, offset)[0]
        offset += 4
        end = offset + size
        if end > len(records) or size < 4:
            malformed += 1
            break
        record = records[offset:end]
        offset = end
        tag = struct.unpack_from("<I", record, 0)[0]
        if tag >= len(TYPE_NAMES):
            malformed += 1
            continue

        kind = TYPE_NAMES[tag]
        cursor = 4
        try:
            if tag == 1:  # Outcome starts with record_type, then symbol.
                _, cursor = read_string(record, cursor)
            symbol, cursor = read_string(record, cursor)
            timestamp, cursor = read_u64(record, cursor)
            counts[kind] += 1
            symbols[(kind, symbol)] += 1
            bounds = timestamp_bounds[kind]
            bounds[0] = timestamp if bounds[0] is None else min(bounds[0], timestamp)
            bounds[1] = timestamp if bounds[1] is None else max(bounds[1], timestamp)
            if tag == 0:
                direction, _ = read_string(record, cursor)
                directions[direction] += 1
        except (IndexError, struct.error, UnicodeError):
            malformed += 1

    return {
        "file_bytes": len(raw),
        "uncompressed_bytes": len(data),
        "records": sum(counts.values()),
        "malformed_records": malformed,
        "record_types": dict(sorted(counts.items())),
        "symbols_by_type": {f"{kind}:{symbol}": count for (kind, symbol), count in sorted(symbols.items())},
        "snapshot_directions": dict(sorted(directions.items())),
        "time_bounds_utc": {
            kind: [timestamp_text(bounds[0]), timestamp_text(bounds[1])]
            for kind, bounds in sorted(timestamp_bounds.items())
        },
    }


def main():
    paths = sorted(SIGNALS_FILE.parent.glob(f"{SIGNALS_FILE.stem}*.{SIGNALS_FILE.suffix.lstrip('.')}"))
    result = {path.name: summarize(path) for path in paths if path.is_file()}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
