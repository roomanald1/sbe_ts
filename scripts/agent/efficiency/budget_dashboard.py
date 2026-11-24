#!/usr/bin/env python3
"""Legacy token summary and deterministic v35 implementation budget ledger."""
import argparse
import fcntl
import hashlib
import json
import os
import sqlite3
import tempfile
from datetime import datetime, timezone
from pathlib import Path

from common import ROOT, session_files, events

PLAN = ROOT / "src/rust/auto_trader/TRADING_IMPROVEMENTS.MD"
BASELINES = {
    "Phase 0": 1_500_000, "Wave 1A": 4_000_000, "Wave 1B": 1_000_000,
    "Wave 2": 8_500_000, "Wave 3": 18_000_000, "Wave 4": 4_500_000,
}
PHASE_START = "2026-11-24T15:43:18Z"  # Beads sbe_ts-gu78 creation time
DEFAULT_STATE = Path.home() / ".cache/sbe-diagnostics/v35-budget-state.json"
DB_PATH = Path.home() / ".codex/state_5.sqlite"
COUNTERS = ("input_tokens", "cached_input_tokens", "output_tokens", "reasoning_output_tokens")


def _atomic_write(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f, indent=2, sort_keys=True)
            f.write("\n")
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def _state(path):
    try:
        return json.loads(Path(path).read_text())
    except FileNotFoundError:
        return {"schema_version": 1, "plan": None, "checkpoints": [], "forecasts": [], "allowance_observations": []}


def _plan_hash():
    return hashlib.sha256(PLAN.read_bytes()).hexdigest()


def init_plan(state):
    if state["plan"]:
        baseline_hash = hashlib.sha256(json.dumps(state["plan"]["baseline"], sort_keys=True).encode()).hexdigest()
        if state["plan"]["version"] != "v35" or state["plan"].get("baseline_sha256") != baseline_hash:
            raise ValueError("Existing baseline is immutable; use an auditable correction, not reinitialization.")
        return state
    state["plan"] = {"version": "v35", "plan_sha256": _plan_hash(), "baseline": BASELINES,
                     "baseline_sha256": hashlib.sha256(json.dumps(BASELINES, sort_keys=True).encode()).hexdigest(),
                     "total_baseline": sum(BASELINES.values()), "original_estimate": 55_000_000,
                     "stretch": 30_000_000, "contingency": 50_000_000, "deferred": ["WP16"],
                     "initialized_at": datetime.now(timezone.utc).isoformat(), "allocations": {},
                     "status": {k: "not_started" for k in BASELINES}}
    return state


def checkpoint(state, record):
    if not state["plan"]:
        raise ValueError("Initialize v35 plan first")
    if not record.get("checkpoint_id"):
        raise ValueError("checkpoint_id is required")
    if any(x["checkpoint_id"] == record["checkpoint_id"] for x in state["checkpoints"]):
        return state, False
    contributions = set(record.get("contribution_ids", []))
    if contributions:
        for prior in state["checkpoints"]:
            prior_ids = set(prior.get("contribution_ids", []))
            overlap = contributions & prior_ids
            if overlap and overlap != contributions:
                raise ValueError("Partially overlapping contribution IDs are ambiguous; split the checkpoint before recording")
            if overlap == contributions:
                record["duplicate_of"] = prior["checkpoint_id"]
                break
    wave = record.get("wave")
    if wave not in state["plan"]["baseline"]:
        raise ValueError("Unknown wave; use Phase 0, Wave 1A, Wave 1B, Wave 2, Wave 3 or Wave 4")
    record.setdefault("timestamp", datetime.now(timezone.utc).isoformat())
    record.setdefault("kind", "manual")
    record.setdefault("counters", {})
    record.setdefault("session_ids", [])
    record.setdefault("provenance", "explicit_manual_checkpoint")
    record.setdefault("allowance", {"status": "unavailable", "reason": "not supplied"})
    state["checkpoints"].append(record)
    return state, True


def allocate(state, allocation):
    wave, slices = allocation.get("wave"), allocation.get("slices") or {}
    if wave not in state.get("plan", {}).get("baseline", {}):
        raise ValueError("Initialize the plan and provide a known wave")
    if not slices or any(int(v) < 0 for v in slices.values()):
        raise ValueError("Each wave allocation needs nonnegative slice budgets")
    if sum(map(int, slices.values())) != state["plan"]["baseline"][wave]:
        raise ValueError("Slice allocations must sum exactly to the immutable wave baseline")
    state["plan"].setdefault("allocations", {})[wave] = {k: int(v) for k, v in slices.items()}
    state["plan"].setdefault("allocation_revisions", []).append({"wave": wave, "slices": dict(slices),
        "reason": allocation.get("reason", "initial allocation"), "recorded_at": datetime.now(timezone.utc).isoformat()})
    return state


def _phase0_telemetry(session_ids, since=PHASE_START):
    """Read selected rollouts and the session index read-only; report coverage, never guess attribution."""
    start = datetime.fromisoformat(since.replace("Z", "+00:00"))
    if start.tzinfo is None:
        start = start.replace(tzinfo=timezone.utc)
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    try:
        rows = conn.execute("select id, rollout_path, archived, tokens_used, attached_thread_parent_id from threads where id in (%s)" % ",".join("?" for _ in session_ids), session_ids).fetchall() if session_ids else []
    finally:
        conn.close()
    by_id = {r[0]: r for r in rows}
    result = {"requested_sessions": list(session_ids), "found_sessions": [], "missing_sessions": [],
              "archived_sessions": [], "ambiguous_sessions": [], "boundary_crossings": [], "missing_metadata": [], "per_session": [], "totals": {k: 0 for k in COUNTERS}, "requests": 0,
              "compactions": 0, "sources": [], "unattributed": {k: 0 for k in COUNTERS},
              "coverage": "partial", "allowance": {"status": "unavailable", "reason": "user directed no allowance inspection"}}
    seen_requests = set()
    for sid in session_ids:
        row = by_id.get(sid)
        if not row:
            result["missing_sessions"].append(sid)
            continue
        _, rollout, archived, db_tokens, parent = row
        result["found_sessions"].append(sid)
        if archived:
            result["archived_sessions"].append(sid)
        path = Path(rollout)
        if not path.exists():
            result["missing_sessions"].append(sid + " (rollout unavailable)")
            continue
        result["sources"].append({"session_id": sid, "rollout": str(path), "archived": bool(archived), "parent_id": parent, "sqlite_tokens_used": db_tokens})
        previous = None
        turn_snapshots = {}
        turn_record_ids = set()
        fallback_totals = {k: 0 for k in COUNTERS}
        fallback_requests = 0
        session_totals = {k: 0 for k in COUNTERS}
        session_requests = 0
        with path.open() as f:
            for line in f:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                ts = event.get("timestamp")
                try:
                    dt = datetime.fromisoformat(ts.replace("Z", "+00:00")) if ts else None
                except (ValueError, AttributeError):
                    dt = None
                if dt and dt.tzinfo is None:
                    dt = dt.replace(tzinfo=timezone.utc)
                payload = event.get("payload") or {}
                etype = event.get("type")
                # token_usage_record is per-response; token_count is cumulative and is a fallback only.
                if etype == "token_usage_record":
                    # Rollouts repeat cumulative turn counters on each response. Keep
                    # the largest counter snapshot per turn; summing snapshots
                    # multiplies usage by the number of tool/assistant messages.
                    if payload.get("thread_id") != sid:
                        continue
                    usage = payload.get("turn_token_usage")
                    turn_id = payload.get("turn_id")
                    rid = payload.get("response_id") or turn_id
                    if not usage or not turn_id or rid in seen_requests:
                        continue
                    seen_requests.add(rid)
                    snap = turn_snapshots.setdefault(turn_id, {k: 0 for k in COUNTERS})
                    for key in COUNTERS:
                        amount = int(usage.get(key, 0) or 0)
                        if dt and dt < start:
                            snap["_before_" + key] = amount
                        else:
                            snap[key] = max(snap[key], amount)
                    turn_record_ids.add(turn_id)
                elif payload.get("type") == "token_count":
                    info = payload.get("info") or {}
                    cumulative = info.get("total_token_usage") or {}
                    marker = json.dumps(cumulative, sort_keys=True)
                    if not cumulative or marker == previous or (dt and dt < start):
                        continue
                    previous = marker
                    last = info.get("last_token_usage") or {}
                    for key in COUNTERS:
                        fallback_totals[key] += int(last.get(key, 0) or 0)
                    fallback_requests += 1
                if etype == "compaction" or payload.get("type") in ("compaction", "context_compacted"):
                    result["compactions"] += 1
        if turn_record_ids:
            # A turn that straddles the requested boundary cannot be split from
            # snapshots alone. Keep it explicit as ambiguous instead of guessing.
            for turn_id, usage in turn_snapshots.items():
                turn_had_post_boundary = any(usage[k] for k in COUNTERS)
                if not turn_had_post_boundary:
                    continue
                result["requests"] += 1
                session_requests += 1
                for key in COUNTERS:
                    before = int(usage.get("_before_" + key, 0))
                    after = usage[key]
                    delta = max(0, after - before)
                    result["totals"][key] += delta
                    session_totals[key] += delta
            if any(any(usage.get("_before_" + k, 0) for k in COUNTERS) for usage in turn_snapshots.values()):
                result["boundary_crossings"].append(sid + " (pre/post snapshots used to calculate exact counter delta)")
        else:
            for key in COUNTERS:
                result["totals"][key] += fallback_totals[key]
            result["requests"] += fallback_requests
            session_totals = fallback_totals
            session_requests = fallback_requests
        if parent is None:
            result["missing_metadata"].append(sid + " (session parent linkage unavailable in SQLite index)")
        result["per_session"].append({"session_id": sid, "parent_session_id": parent,
            "sqlite_lifetime_tokens_used": db_tokens, "phase_window_token_counters": session_totals,
            "requests": session_requests, "archived": bool(archived)})
    result["coverage"] = "partial" if result["missing_sessions"] or result["ambiguous_sessions"] else "complete_for_requested_sources"
    result["scope"] = {"since": start.isoformat(), "source_sessions": len(result["found_sessions"]), "policy": "explicit session IDs; no wall-clock-only attribution"}
    return result


def summary(state):
    totals = {k: 0 for k in COUNTERS}
    slices = {}
    unattributed = {k: 0 for k in COUNTERS}
    for cp in state["checkpoints"]:
        if cp.get("kind") == "start" or cp.get("duplicate_of"):
            continue
        values = cp.get("counters") or {}
        # A checkpoint may carry explicit deltas, or monotonic start/end snapshots.
        deltas = values.get("delta")
        if deltas is None and cp.get("kind") == "end" and cp.get("start_counters"):
            deltas = {k: max(0, int(values.get(k, 0))-int(cp["start_counters"].get(k, 0))) for k in COUNTERS}
        if deltas is None:
            deltas = {k: 0 for k in COUNTERS}
            for k in COUNTERS:
                if k in values:
                    deltas[k] = int(values[k])
        wave = cp.get("wave", "unassigned")
        target = slices.setdefault(wave, {k: 0 for k in COUNTERS})
        for k in COUNTERS:
            target[k] += int(deltas.get(k, 0) or 0)
            totals[k] += int(deltas.get(k, 0) or 0)
            if cp.get("attribution") in ("ambiguous", "unattributed", "missing"):
                unattributed[k] += int(deltas.get(k, 0) or 0)
    waves = {}
    for wave, baseline in (state.get("plan") or {}).get("baseline", {}).items():
        actual = slices.get(wave, {}).get("input_tokens", 0)
        slice_report = {}
        allocation = (state["plan"].get("allocations") or {}).get(wave, {})
        for name, slice_budget in allocation.items():
            slice_actual = sum(int((cp.get("counters", {}).get("delta") or {}).get("input_tokens", 0))
                               for cp in state["checkpoints"] if cp.get("wave") == wave and cp.get("slice") == name and not cp.get("duplicate_of") and cp.get("kind") != "start")
            slice_report[name] = {"baseline": slice_budget, "actual": slice_actual,
                                  "variance": slice_actual-slice_budget, "remaining": slice_budget-slice_actual}
        waves[wave] = {"baseline": baseline, "actual": actual, "variance": actual-baseline,
                       "remaining": baseline-actual, "status": (state["plan"].get("status") or {}).get(wave, "not_started")}
        waves[wave]["slices"] = slice_report
    total_baseline = (state.get("plan") or {}).get("total_baseline", 0)
    actual = totals["input_tokens"]
    forecasts = state.get("forecasts", [])
    return {"version": "v35", "plan": state.get("plan"), "totals": totals, "waves": waves,
            "cumulative_actual_input": actual, "total_baseline": total_baseline,
            "total_remaining": total_baseline-actual, "unattributed": unattributed,
            "forecast": forecasts[-1] if forecasts else {"status": "not_estimated", "assumptions": "No remaining-work estimate recorded"},
            "checkpoint_count": len(state["checkpoints"]), "allowance": {"status": "unavailable", "reason": "user directed no allowance inspection"}}


def legacy(since, budget):
    since_dt = datetime.fromisoformat(since.replace("Z", "+00:00"))
    if since_dt.tzinfo is None:
        since_dt = since_dt.replace(tzinfo=timezone.utc)
    totals = {k: 0 for k in COUNTERS}
    requests, seen = [], set()
    for path in session_files():
        if path.stat().st_mtime < since_dt.timestamp():
            continue
        sid, matched, previous = None, False, None
        for event in events(path):
            payload = event.get("payload") or {}
            if event.get("type") == "session_meta":
                sid = payload.get("id")
                cwd = payload.get("cwd")
                matched = bool(cwd and Path(cwd).is_relative_to(ROOT))
                if sid in seen:
                    break
                if matched and sid:
                    seen.add(sid)
            if not matched or payload.get("type") != "token_count":
                continue
            info = payload.get("info") or {}
            cumulative, last = info.get("total_token_usage"), info.get("last_token_usage")
            marker = json.dumps(cumulative, sort_keys=True)
            if not cumulative or not last or marker == previous:
                continue
            previous = marker
            try:
                ts = datetime.fromisoformat(event["timestamp"].replace("Z", "+00:00"))
            except (KeyError, ValueError):
                continue
            if ts < since_dt:
                continue
            for key in totals:
                totals[key] += last.get(key, 0)
            requests.append(last.get("input_tokens", 0))
    used, cap = totals["input_tokens"], budget * 1_000_000
    if cap <= 0:
        raise ValueError("Budget must be positive")
    return {"since": since_dt.isoformat(), "sessions": len(seen), "requests": len(requests), "totals": totals,
            "input_budget": int(cap), "budget_used_percent": round(used/cap*100, 2),
            "mean_request_input": round(sum(requests)/len(requests)) if requests else 0,
            "max_request_input": max(requests, default=0),
            "cache_percent": round(totals["cached_input_tokens"]/used*100, 2) if used else 0,
            "limitations": ["Active session directory only.", "Repository and date scoped, not work-package scoped.", "Historical token events may contain resets or forks.", "Not a measurement of Codex subscription allowance."]}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--since", help="Legacy ISO timestamp/date summary")
    p.add_argument("--budget", type=float, default=45, help="Legacy budget in millions of input tokens")
    p.add_argument("--state", type=Path, default=DEFAULT_STATE, help=f"Atomic ledger file (default: {DEFAULT_STATE})")
    p.add_argument("--init-plan", metavar="VERSION", help="Initialize immutable plan baseline (v35)")
    p.add_argument("--start-wave", metavar="WAVE")
    p.add_argument("--complete-wave", metavar="WAVE")
    p.add_argument("--checkpoint", action="store_true", help="Write an explicit checkpoint from --record JSON")
    p.add_argument("--record", help="Checkpoint JSON object or @path; requires checkpoint_id and wave")
    p.add_argument("--plan-summary", action="store_true")
    p.add_argument("--forecast", help="Record forecast JSON: {input_tokens,reason,uncertainty}")
    p.add_argument("--allocate", help="Allocate a wave baseline to slices: {wave,slices,reason}")
    p.add_argument("--reconcile-sessions", nargs="*", help="Read-only reconciliation for explicit SQLite session IDs")
    p.add_argument("--phase0-sessions", action="store_true", help="Reconcile known Phase 0 root and child session IDs")
    p.add_argument("--phase0-since", default=PHASE_START, help="Phase 0 counter boundary (UTC ISO timestamp)")
    p.add_argument("--output", type=Path, help="Write reconciliation evidence JSON to this path")
    p.add_argument("--help-v35", action="store_true", help="Show v35 command examples and telemetry policy")
    a = p.parse_args()
    if a.help_v35:
        print("v35 examples:\n  --init-plan v35\n  --start-wave 'Phase 0'\n  --checkpoint --record '{...}'\n  --plan-summary\n  --reconcile-sessions ID [ID ...]\nLegacy: --since ISO [--budget MILLIONS]\nAllowance is separate and never inferred from tokens.")
        return
    if a.since:
        print(json.dumps(legacy(a.since, a.budget), indent=2))
        return
    lock_path = a.state.with_suffix(a.state.suffix + ".lock")
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    lock_file = lock_path.open("a+")
    fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
    state = _state(a.state)
    if a.init_plan:
        if a.init_plan != "v35":
            p.error("Only immutable baseline v35 is defined")
        _atomic_write(a.state, init_plan(state))
    elif a.start_wave or a.complete_wave:
        if not state.get("plan"):
            p.error("Initialize plan first")
        wave = a.start_wave or a.complete_wave
        if wave not in state["plan"]["status"]:
            p.error("Unknown wave")
        state["plan"]["status"][wave] = "in_progress" if a.start_wave else "complete"
        _atomic_write(a.state, state)
    elif a.checkpoint:
        if not a.record:
            p.error("--checkpoint requires --record JSON or @path")
        raw = a.record[1:] if a.record.startswith("@") else None
        record = json.loads(Path(raw).read_text() if raw else a.record)
        state, added = checkpoint(state, record)
        _atomic_write(a.state, state)
        print(json.dumps({"checkpoint_added": added, "checkpoint_id": record["checkpoint_id"]}))
    elif a.forecast:
        if not state.get("plan"):
            p.error("Initialize plan first")
        forecast = json.loads(a.forecast)
        forecast["recorded_at"] = datetime.now(timezone.utc).isoformat()
        forecast["status"] = "estimate_with_uncertainty" if forecast.get("uncertainty") else "estimate_uncertainty_unspecified"
        state["forecasts"].append(forecast)
        _atomic_write(a.state, state)
    elif a.allocate:
        try:
            allocate(state, json.loads(a.allocate))
        except ValueError as e:
            p.error(str(e))
        _atomic_write(a.state, state)
    elif a.reconcile_sessions is not None or a.phase0_sessions:
        ids = a.reconcile_sessions if a.reconcile_sessions is not None else [
            "01a20e0c-5c94-7272-b1ed-68393025cf84", "01a20e29-e10f-7353-9c4e-265b4f3a1a25",
            "01a20e4a-2e33-7ca2-967f-3405efbb5c17", "01a20e3d-a2c2-7793-8fe0-896453823b49",
            "01a20e40-d217-7553-ae5c-3b4dec581fef", "01a20e46-9a9a-70c2-babd-bf51563197b2",
            "01a20e4a-2c67-7ce0-a13f-21235a30586e"]
        result = _phase0_telemetry(ids, a.phase0_since)
        rendered = json.dumps(result, indent=2)
        if a.output:
            _atomic_write(a.output, result)
        print(rendered)
    elif a.plan_summary:
        print(json.dumps(summary(state), indent=2))
    else:
        p.print_help()


if __name__ == "__main__":
    main()
