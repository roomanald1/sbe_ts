#!/usr/bin/env python3
import json
import math
from collections import Counter
from pathlib import Path

from common import parser, load_rows, emit

p = parser("Report learning evidence gaps")
a = p.parse_args()


def numeric(value):
    return (isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value) and value >= 0)


def coverage_rows_from_api(body):
    """Normalize actual AutoTrader decision-parameters evidence without making missing equal zero."""
    as_of = body.get("refreshed_at")
    readiness = body.get("evidence_readiness")
    if isinstance(readiness, list):
        return [{
            "strategy": row.get("strategy"),
            "symbol": row.get("symbol"),
            "subject_type": row.get("subject_type", "profile"),
            "subject_id": row.get("subject_id"),
            "effective_samples": row.get("effective_samples"),
            "required_effective_samples": row.get("required_samples"),
            "status": "ready" if not row.get("promotion_blockers") else "blocked",
            "evidence_present": row.get("evidence_present"),
            "age_ms": row.get("age_ms"),
            "stale": row.get("stale"),
            "history_coverage": row.get("history_coverage"),
            "cost_coverage": row.get("cost_coverage"),
            "search_family_coverage": row.get("search_family_coverage"),
            "queued_work": row.get("queued_work"),
            "completed_work": row.get("completed_work"),
            "promotion_blockers": row.get("promotion_blockers", []),
            "as_of_ms": as_of,
        } for row in readiness]

    profiles = body.get("strategy_profiles")
    if not isinstance(profiles, list):
        return None
    rows = []
    for profile in profiles:
        params = profile.get("parameters") or {}
        if isinstance(params, list):
            params = {
                row.get("key"): row.get("value")
                for row in params if isinstance(row, dict) and isinstance(row.get("key"), str)
            }
        if not isinstance(params, dict):
            params = {}
        required = params.get("scout_minimum_walk_forward_samples")
        max_age_seconds = params.get("scout_maximum_evidence_age_seconds")
        evidence = profile.get("walk_forward_evidence")
        blockers = []
        if evidence is None:
            blockers.append("evidence_missing")
            sample_count = effective = age_ms = stale = None
            history = cost = search = None
            holdout = scope = lower_bound = None
        else:
            sample_count = evidence.get("sampleCount")
            effective = evidence.get("effectiveSampleSize")
            last_sample = evidence.get("lastSampleTimestampMs")
            age_ms = max(0, int(as_of) - int(last_sample)) if numeric(as_of) and numeric(last_sample) else None
            stale = age_ms > max_age_seconds * 1000 if age_ms is not None and numeric(max_age_seconds) and max_age_seconds > 0 else None
            history = evidence.get("historyCoverage")
            cost = evidence.get("costCoverage")
            search = evidence.get("searchFamilyCoverage")
            holdout = evidence.get("finalHoldoutStatus")
            scope = evidence.get("evidenceScope")
            lower_bound = evidence.get("expectancyCiLowPercent")
            if sample_count is None:
                blockers.append("sample_count_unavailable")
            if effective is None:
                blockers.append("effective_samples_unavailable")
            if stale is None:
                blockers.append("freshness_threshold_unavailable")
            elif stale:
                blockers.append("evidence_stale")
            if history != "complete":
                blockers.append("history_coverage_incomplete_or_unknown")
            if cost != "complete":
                blockers.append("cost_coverage_incomplete_or_unknown")
            if search != "complete":
                blockers.append("search_family_coverage_incomplete_or_unknown")
            if holdout != "validated":
                blockers.append("final_holdout_not_validated")
            if not numeric(lower_bound) or lower_bound <= 0:
                blockers.append("positive_expectancy_not_proven")
            if scope != "exact_profile_unconditioned":
                blockers.append("evidence_not_exact_profile")
        if not numeric(required) or required <= 0:
            blockers.append("required_sample_threshold_unavailable")
        else:
            if numeric(sample_count) and sample_count < required:
                blockers.append("insufficient_samples")
            if numeric(effective) and effective < required:
                blockers.append("insufficient_effective_samples")
        rows.append({
            "strategy": profile.get("strategy"), "symbol": profile.get("symbol"),
            "subject_type": "profile", "subject_id": f"{profile.get('symbol')}:{profile.get('strategy')}",
            "effective_samples": effective, "required_effective_samples": required,
            "status": "ready" if not blockers else ("missing" if evidence is None else "blocked"),
            "evidence_present": evidence is not None, "age_ms": age_ms, "stale": stale,
            "history_coverage": history, "cost_coverage": cost,
            "search_family_coverage": search, "queued_work": None, "completed_work": None,
            "promotion_blockers": sorted(set(blockers)), "as_of_ms": as_of,
        })

    # Current context aggregates have Scout counts but no contextual walk-forward
    # evidence or work totals. Preserve those raw counts and mark the missing fields.
    for context in body.get("contexts") or []:
        parts = str(context.get("context", "")).split(":")
        symbol = parts[2] if len(parts) >= 3 else None
        strategy = parts[3] if len(parts) >= 4 else None
        rows.append({
            "strategy": strategy, "symbol": symbol, "subject_type": "context",
            "subject_id": context.get("context"), "effective_samples": None,
            "required_effective_samples": None, "status": "missing",
            "evidence_present": False, "raw_learning_samples": context.get("learning_samples"),
            "raw_scout_samples": context.get("scout_learning_samples"), "age_ms": None,
            "stale": None, "history_coverage": None, "cost_coverage": None,
            "search_family_coverage": None, "queued_work": None, "completed_work": None,
            "promotion_blockers": ["context_walk_forward_evidence_unavailable", "work_counts_unavailable"],
            "as_of_ms": as_of,
        })
    return rows


def load_coverage_rows(path, max_rows):
    if path is None:
        raise SystemExit("BLOCKED: supply a verified data export with --input")
    if path.suffix.lower() == ".json":
        raw = json.loads(path.read_text())
        body = raw.get("body") if isinstance(raw, dict) else None
        if isinstance(body, dict):
            normalized = coverage_rows_from_api(body)
            if normalized is not None:
                return normalized[:max_rows]
    return load_rows(path, max_rows)


rows = load_coverage_rows(a.input, a.max_rows)
if not rows:
    raise SystemExit("BLOCKED: no coverage rows in input")
if any(
    not isinstance(row.get("subject_id"), str)
    if row.get("subject_type") == "context"
    else not isinstance(row.get("strategy"), str) or not isinstance(row.get("symbol"), str)
    for row in rows
):
    raise SystemExit("BLOCKED: coverage row missing strategy/symbol identity")

gaps = []

for r in rows:
    actual = r.get("effective_samples")
    required = r.get("required_effective_samples")
    if actual is not None and (not numeric(actual)):
        raise SystemExit("BLOCKED: invalid effective sample count")
    if required is not None and (not numeric(required) or required <= 0):
        raise SystemExit("BLOCKED: invalid required effective sample count")
    blockers = r.get("promotion_blockers") or []
    if r.get("status") != "ready" or blockers:
        deficit = max(0, required-actual) if numeric(actual) and numeric(required) else None
        coverage = min(1, actual/required) if numeric(actual) and numeric(required) and required > 0 else None
        gaps.append({**r, "sample_deficit": deficit, "coverage": coverage})

gaps.sort(key=lambda r: (r["coverage"] is not None, r["coverage"] if r["coverage"] is not None else -1))

emit(a, {
    "profiles": sum(row.get("subject_type", "profile") == "profile" for row in rows),
    "contexts": sum(row.get("subject_type") == "context" for row in rows),
    "subjects": len(rows),
    "profiles_with_gaps": sum(row.get("subject_type", "profile") == "profile" and row.get("status") != "ready" for row in rows),
    "contexts_with_gaps": sum(row.get("subject_type") == "context" and row.get("status") != "ready" for row in rows),
    "ready": sum(row.get("subject_type", "profile") == "profile" and row.get("status") == "ready" for row in rows),
    "missing_evidence": sum(not row.get("evidence_present", True) for row in rows),
    "zero_effective_sample_rows": sum(row.get("effective_samples") == 0 for row in rows),
    "unknown_effective_sample_rows": sum(row.get("effective_samples") is None for row in rows),
    "stale_rows": sum(row.get("stale") is True for row in rows),
    "blocker_counts": dict(Counter(blocker for row in rows for blocker in row.get("promotion_blockers", []))),
    "source": str(a.input),
    "gaps": gaps[:a.limit]
}, gaps)
