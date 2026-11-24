import json
import sqlite3
from pathlib import Path
from datetime import datetime, timezone

def rollout_files(hours=24, limit=100):
    root = Path.home() / ".codex"
    cutoff = datetime.now(timezone.utc).timestamp() - hours * 3600
    paths = []
    for folder in ("sessions", "archived_sessions"):
        for path in (root / folder).rglob("*.jsonl"):
            try:
                if path.stat().st_mtime >= cutoff:
                    paths.append(path)
            except OSError:
                pass
    return sorted(
        paths, key=lambda p: p.stat().st_mtime, reverse=True
    )[:limit]

def events(path):
    with path.open(errors="replace") as f:
        for line in f:
            try:
                event = json.loads(line)
                if isinstance(event, dict):
                    yield event
            except ValueError:
                continue

def thread_metadata():
    db = Path.home() / ".codex/state_5.sqlite"
    if not db.exists():
        return {}
    result = {}
    try:
        with sqlite3.connect(
            db.resolve().as_uri() + "?mode=ro",
            uri=True, timeout=5
        ) as conn:
            conn.row_factory = sqlite3.Row
            cols = {
                r[1] for r in conn.execute("PRAGMA table_info(threads)")
            }
            wanted = [
                "id", "rollout_path", "agent_nickname",
                "agent_role", "agent_path", "model",
                "reasoning_effort", "model_provider", "cli_version"
            ]
            selected = [c for c in wanted if c in cols]
            if "rollout_path" not in selected:
                return {}
            for row in conn.execute(
                "SELECT " + ",".join(selected) + " FROM threads"
            ):
                item = dict(row)
                if item.get("rollout_path"):
                    result[item["rollout_path"]] = item
    except sqlite3.Error:
        pass
    return result

def usage(path, hours=None):
    calls = []
    previous = None
    cutoff = (
        datetime.now(timezone.utc).timestamp() - hours * 3600
        if hours is not None else None
    )
    for event in events(path):
        payload = event.get("payload") or {}
        if not isinstance(payload, dict):
            continue
        if payload.get("type") != "token_count":
            continue
        info = payload.get("info") or {}
        total = info.get("total_token_usage") or {}
        last = info.get("last_token_usage") or {}
        if not total or total == previous:
            continue
        previous = total.copy()
        if not last:
            continue
        timestamp = event.get("timestamp", "")
        if cutoff is not None:
            try:
                ts = datetime.fromisoformat(
                    timestamp.replace("Z", "+00:00")
                ).timestamp()
                if ts < cutoff:
                    continue
            except (ValueError, TypeError):
                continue
        calls.append({
            "timestamp": timestamp,
            "input": int(last.get("input_tokens", 0)),
            "cached": int(last.get("cached_input_tokens", 0)),
            "output": int(last.get("output_tokens", 0)),
            "reasoning": int(last.get("reasoning_output_tokens", 0)),
        })
    return calls
