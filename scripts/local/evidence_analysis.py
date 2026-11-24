import argparse
import json
import math
import subprocess
from collections import Counter
from pathlib import Path
from datetime import datetime, timezone

from common import EVIDENCE, ROOT, emit, save, now

def latest(prefix):
    files = sorted(
        EVIDENCE.glob(prefix+"-*.json"),
        key=lambda p: p.stat().st_mtime,
        reverse=True
    )
    if not files:
        raise RuntimeError("No evidence for "+prefix)
    return files[0]

def load(path):
    path = Path(path)
    if path.stat().st_size > 16000000:
        raise RuntimeError("Evidence exceeds 16 MB")
    return json.loads(path.read_text())

def body(path):
    data = load(path)
    if "body" not in data:
        raise RuntimeError("API evidence body missing")
    return data["body"]

def number(value):
    return (
        isinstance(value, (int,float))
        and not isinstance(value,bool)
        and math.isfinite(value)
    )

def age(path):
    return round(
        datetime.now(timezone.utc).timestamp()
        - Path(path).stat().st_mtime
    )

def feedback(path):
    data = body(path)
    capacity = data.get("feedback_capacity")
    depth = data.get("feedback_depth")
    queues = []
    for key,value in data.items():
        if not key.endswith("_depth"):
            continue
        prefix = key[:-6]
        oldest = data.get(prefix+"_oldest_age_ms")
        queues.append({
            "queue": prefix,
            "depth": value,
            "oldest_minutes": round(oldest/60000,2)
                              if number(oldest) else None
        })

    lifecycle = []
    for name,stats in data.get("lifecycle",{}).items():
        lifecycle.append({
            "queue": name,
            "attempts": stats.get("attempts"),
            "acknowledged": stats.get("acknowledged"),
            "rejected": stats.get("rejected"),
            "permanent_rejections": stats.get(
                "permanent_rejections"
            ),
            "evicted_oldest": stats.get("evicted_oldest"),
            "terminal_drops": stats.get("terminal_drops"),
            "transient_errors": stats.get("transient_errors"),
            "latest_reason": stats.get("latest_reason"),
            "latest_reason_at_ms": stats.get(
                "latest_reason_at_ms"
            )
        })

    return {
        "status": "OBSERVED",
        "source": str(path),
        "evidence_age_seconds": age(path),
        "feedback_capacity": capacity,
        "feedback_depth": depth,
        "capacity_used_percent": round(100*depth/capacity,2)
            if number(capacity) and capacity > 0
            and number(depth) else None,
        "queues": queues,
        "lifecycle": lifecycle,
        "note": "Counters are cumulative; reset epoch unverified."
    }

def feedback_delta(old_path,new_path):
    old = body(old_path)
    new = body(new_path)
    elapsed = (
        Path(new_path).stat().st_mtime
        - Path(old_path).stat().st_mtime
    )
    if elapsed <= 0:
        raise RuntimeError("Snapshots must be chronological")

    result = []
    counters = [
        "attempts","acknowledged","rejected",
        "permanent_rejections","evicted_oldest",
        "terminal_drops","transient_errors"
    ]

    for name,current in new.get("lifecycle",{}).items():
        previous = old.get("lifecycle",{}).get(name,{})
        deltas = {}
        decreases = []
        for field in counters:
            x,y = previous.get(field),current.get(field)
            if number(x) and number(y):
                if y < x:
                    decreases.append(field)
                else:
                    deltas[field] = y-x

        result.append({
            "queue": name,
            "counter_deltas": deltas if not decreases else None,
            "possible_reset": bool(decreases),
            "decreased_counters": decreases,
            "latest_reason": current.get("latest_reason")
        })

    return {
        "status":"PROVISIONAL",
        "elapsed_seconds":round(elapsed,2),
        "old":str(old_path),
        "new":str(new_path),
        "queues":result,
        "note":"No restart/epoch proof. Deltas are provisional."
    }

def learning(path):
    rows = body(path)
    if not isinstance(rows,list):
        raise RuntimeError("Unexpected learning summary shape")
    result = []

    for row in rows:
        total = row.get("learning_samples")
        active = row.get("active_profile_trade_samples")
        other = row.get("other_profile_trade_samples")
        rate = row.get("paired_scout_positive_rate")
        lift = row.get("average_paired_scout_lift_percent")
        result.append({
            "strategy":row.get("strategy"),
            "learning_samples":total,
            "active_samples":active,
            "other_samples":other,
            "active_share_percent":round(100*active/total,2)
                if number(total) and total>0
                and number(active) else None,
            "sample_count_consistent":total==active+other
                if all(number(x) for x in (total,active,other))
                else None,
            "scout_samples":row.get("scout_outcome_samples"),
            "paired_samples":row.get("paired_scout_samples"),
            "paired_positive_rate_raw":rate,
            "paired_lift_percent_raw":lift,
            "promotion_status":"NOT_EVALUATED"
        })

    return {
        "source":str(path),
        "evidence_age_seconds":age(path),
        "strategies":result,
        "note":"Raw counts only; independence, freshness, "
               "profile epoch and promotion gates unverified."
    }

def profiles(path):
    rows = body(path)
    if not isinstance(rows,list):
        raise RuntimeError("Unexpected profile response")
    counts = {}
    ids = Counter()

    for row in rows:
        strategy = row.get("strategy","UNKNOWN")
        entry = counts.setdefault(strategy,{
            "profiles":0,
            "symbols":set(),
            "zero_sample_profiles":0,
            "missing_sample_counts":0
        })
        entry["profiles"] += 1
        if isinstance(row.get("symbol"),str):
            entry["symbols"].add(row["symbol"])
        samples = row.get("learning_samples")
        if samples == 0:
            entry["zero_sample_profiles"] += 1
        elif not number(samples):
            entry["missing_sample_counts"] += 1
        if isinstance(row.get("profile_id"),str):
            ids[row["profile_id"]] += 1

    for entry in counts.values():
        entry["unique_symbols"] = len(entry.pop("symbols"))

    return {
        "source":str(path),
        "profiles":len(rows),
        "strategies":counts,
        "repeated_profile_id_values":sum(
            1 for n in ids.values() if n>1
        ),
        "note":"Repeated IDs are observations, not proven defects."
    }

def shape(value,depth=0):
    if depth>=4:
        return type(value).__name__
    if isinstance(value,dict):
        return {
            k:shape(v,depth+1)
            for k,v in list(value.items())[:50]
        }
    if isinstance(value,list):
        return {
            "length":len(value),
            "first_item":shape(value[0],depth+1)
                         if value else None
        }
    return type(value).__name__

def financial_inventory():
    result = []
    for endpoint in [
        "account","positions","realized-pnl",
        "fee-summary","fee-rates"
    ]:
        try:
            path = latest("positions-"+endpoint)
            data = body(path)
            result.append({
                "endpoint":endpoint,
                "source":str(path),
                "age_seconds":age(path),
                "shape":shape(data)
            })
        except Exception as e:
            result.append({
                "endpoint":endpoint,
                "status":"BLOCKED",
                "reason":str(e)
            })

    return {
        "status":"CONTRACT_DISCOVERY",
        "sources":result,
        "financial_calculations":"NOT_PERFORMED"
    }

def source_trace(term,limit=80):
    result = subprocess.run(
        [
            "git","grep","-n","-F","-e",term,"--",
            "src/rust/learning",
            "src/rust/auto_trader",
            "src/rust/insights",
            "src/rust/position_management"
        ],
        cwd=ROOT,capture_output=True,text=True,
        timeout=20
    )
    if result.returncode not in (0,1):
        raise RuntimeError(result.stderr[:500])
    lines = result.stdout.splitlines()
    return {
        "term":term,
        "matches":len(lines),
        "locations":[x[:500] for x in lines[:limit]],
        "truncated":len(lines)>limit,
        "scope":"Tracked Rust service files"
    }

def evidence_index():
    groups = {}
    total = 0
    for p in EVIDENCE.glob("*.json"):
        size = p.stat().st_size
        total += size
        prefix = p.stem.rsplit("-",1)[0]
        item = groups.setdefault(prefix,{
            "files":0,"bytes":0
        })
        item["files"] += 1
        item["bytes"] += size
    return {
        "directory":str(EVIDENCE),
        "total_json_bytes":total,
        "groups":groups
    }

def main(name):
    p = argparse.ArgumentParser(description=name)
    p.add_argument("--input")
    p.add_argument("--baseline")
    p.add_argument("--current")
    p.add_argument("--term")
    a = p.parse_args()

    try:
        if name=="feedback_analysis":
            data = feedback(
                a.input or latest("autotrader-feedback-queues")
            )

        elif name=="feedback_delta":
            if not a.baseline or not a.current:
                p.error("--baseline and --current required")
            data = feedback_delta(a.baseline,a.current)

        elif name=="learning_analysis":
            data = learning(
                a.input or latest("learning-summary")
            )

        elif name=="profile_inventory":
            data = profiles(
                a.input or latest("learning-profiles")
            )

        elif name=="financial_contract_inventory":
            data = financial_inventory()

        elif name=="queue_source_trace":
            data = {
                "traces":[source_trace(term,30) for term in [
                    "Learning missed-opportunity queue is full",
                    "permanent_rejections",
                    "evicted_oldest",
                    "terminal_drops",
                    "missed_opportunity_depth"
                ]]
            }

        elif name=="source_trace":
            if not a.term:
                p.error("--term required")
            data = source_trace(a.term)

        elif name=="evidence_index":
            data = evidence_index()

        else:
            raise ValueError(name)

        data["generated_at"] = now()
        path = save(data,name)
        emit({"evidence":path,"result":data})
        return 0

    except Exception as e:
        emit({"status":"BLOCKED","error":str(e)})
        return 1
