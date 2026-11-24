
import argparse
import json
import math
from pathlib import Path

from common import save, now

LIMITS = {
    "raw_market_ingress": {
        "p50_ms": 2.0,
        "p95_ms": 5.0,
        "p99_ms": 10.0
    },
    "precomputed_signal_ingress": {
        "p50_ms": 2.0,
        "p95_ms": 5.0,
        "p99_ms": 10.0
    },
    "emergency_ioc_stop": {
        "p99_ms": 2.0
    }
}

STRETCH = {
    "raw_market_ingress": {
        "p50_ms": 0.5,
        "p95_ms": 2.0,
        "p99_ms": 5.0
    },
    "precomputed_signal_ingress": {
        "p50_ms": 0.5,
        "p95_ms": 2.0,
        "p99_ms": 5.0
    },
    "emergency_ioc_stop": {
        "p99_ms": 1.0
    }
}

METADATA = [
    "hardware",
    "git_head",
    "build_mode",
    "workload_id",
    "measurement_definition",
    "benchmark_config_sha256",
    "sampling_method"
]

def numeric(value):
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value >= 0
    )

def evaluate(data):
    checks = []
    blockers = []

    for key in METADATA:
        if not data.get(key):
            blockers.append("Missing metadata: " + key)

    if data.get("build_mode") != "release":
        blockers.append("Release build required")

    rate = data.get("signals_per_second")

    if not numeric(rate) or rate < 100:
        blockers.append(
            "Representative sustained 100 signals/sec "
            "workload not established"
        )

    if data.get("sampling_method") != "all_eligible_events":
        blockers.append(
            "Success-only or unverified sampling"
        )

    measurements = data.get("measurements", {})

    if not isinstance(measurements, dict):
        raise ValueError("Invalid measurements")

    for stage, limits in LIMITS.items():
        metrics = measurements.get(stage)

        if not isinstance(metrics, dict):
            blockers.append("Missing stage: " + stage)
            continue

        count = metrics.get("sample_count")

        if (
            not isinstance(count, int)
            or isinstance(count, bool)
            or count <= 0
        ):
            blockers.append("Missing sample count: " + stage)

        ordered = [
            metrics.get(k)
            for k in [
                "p50_ms", "p95_ms",
                "p99_ms", "p999_ms"
            ]
        ]

        if not all(numeric(x) for x in ordered):
            blockers.append(
                "Missing or invalid percentiles: " + stage
            )
        elif ordered != sorted(ordered):
            blockers.append(
                "Non-monotonic percentiles: " + stage
            )

        for metric, limit in limits.items():
            value = metrics.get(metric)

            if not numeric(value):
                blockers.append(
                    f"Invalid metric: {stage}.{metric}"
                )
                continue

            checks.append({
                "stage": stage,
                "metric": metric,
                "measured_ms": value,
                "acceptance_ms": limit,
                "stretch_ms": STRETCH[stage][metric],
                "status": "PASS" if value <= limit else "FAIL",
                "stretch_met": value <= STRETCH[stage][metric]
            })

    failed = sum(c["status"] == "FAIL" for c in checks)

    status = (
        "THRESHOLD_FAIL" if failed
        else "BLOCKED" if blockers
        else "THRESHOLDS_MET"
    )

    return {
        "timestamp": now(),
        "status": status,
        "checks": checks,
        "blockers": blockers,
        "failed": failed,
        "acceptance": "NOT_EVALUATED",
        "remaining_review": [
            "Measurement provenance",
            "Statistical sample adequacy",
            "Coordinated omission",
            "Representative burst workloads",
            "Diagnostic saturation and scout workload",
            "Baseline workload equivalence",
            "Resource and queue bounds",
            "Protective exit correctness",
            "Coordinator approval"
        ]
    }

def self_test():
    fixture = {
        key: "synthetic-test" for key in METADATA
    }

    fixture.update({
        "build_mode": "release",
        "sampling_method": "all_eligible_events",
        "signals_per_second": 100,
        "measurements": {
            stage: {
                "sample_count": 10000,
                "p50_ms": 0.5,
                "p95_ms": 1.0,
                "p99_ms": 2.0,
                "p999_ms": 4.0
            }
            for stage in LIMITS
        }
    })

    assert evaluate(fixture)["status"] == "THRESHOLDS_MET"

    fixture["measurements"]["emergency_ioc_stop"]["p99_ms"] = 2.01
    assert evaluate(fixture)["status"] == "THRESHOLD_FAIL"

    fixture["measurements"]["emergency_ioc_stop"]["p99_ms"] = 2.0
    fixture["signals_per_second"] = 99
    assert evaluate(fixture)["status"] == "BLOCKED"

    fixture["signals_per_second"] = 100
    del fixture["measurements"]["raw_market_ingress"]["p999_ms"]
    assert evaluate(fixture)["status"] == "BLOCKED"

    print("SELF TEST: 4/4 PASS")
    print("Synthetic fixtures only; no production evidence.")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    if not args.input:
        parser.error("--input required")

    data = json.loads(args.input.read_text())
    result = evaluate(data)
    result["input"] = str(args.input.resolve())

    path = save(result, "performance_gate")

    print(json.dumps({
        "status": result["status"],
        "checks": len(result["checks"]),
        "failed": result["failed"],
        "blockers": result["blockers"],
        "evidence": str(path),
        "acceptance": "NOT_EVALUATED"
    }, indent=2))

    return 0 if result["status"] == "THRESHOLDS_MET" else 1

if __name__ == "__main__":
    raise SystemExit(main())
