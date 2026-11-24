import argparse
from common import collect, compact, emit, save, now

REPORTS = {
    "trading_health": [
        ("learning", "summary"),
        ("autotrader", "feedback-queues"),
        ("autotrader", "open-position-counts"),
        ("positions", "account"),
        ("positions", "fee-summary")
    ],
    "learning_readiness": [
        ("learning", "summary"),
        ("learning", "profiles")
    ],
    "feedback_health": [
        ("autotrader", "feedback-queues")
    ],
    "entry_starvation": [
        ("insights", "dashboard"),
        ("autotrader", "open-position-counts")
    ],
    "pnl_reconciliation": [
        ("positions", "realized-pnl"),
        ("positions", "fee-summary"),
        ("positions", "positions")
    ],
    "scout_comparison": [
        ("learning", "summary"),
        ("learning", "profiles")
    ],
    "capital_efficiency": [
        ("positions", "account"),
        ("positions", "positions"),
        ("autotrader", "open-position-counts")
    ],
    "trade_lifecycle": [
        ("positions", "positions"),
        ("positions", "realized-pnl")
    ],
    "fee_drag": [
        ("positions", "fee-summary"),
        ("positions", "realized-pnl")
    ],
    "strategy_scorecard": [
        ("learning", "summary"),
        ("positions", "realized-pnl")
    ]
}

def report(name):
    sources = [
        collect(service, endpoint)
        for service, endpoint in REPORTS[name]
    ]
    output = {
        "report": name,
        "timestamp": now(),
        "status": (
            "BLOCKED" if any(
                s["status"] == "BLOCKED" for s in sources
            ) else "COLLECTED"
        ),
        "sources": [compact(s) for s in sources],
        "observations": {}
    }

    for source in sources:
        body = source.get("body")
        endpoint = source["endpoint"]

        if endpoint == "summary" and isinstance(body, list):
            fields = [
                "strategy",
                "learning_samples",
                "active_profile_trade_samples",
                "other_profile_trade_samples",
                "scout_outcome_samples",
                "paired_scout_samples",
                "paired_scout_positive_rate",
                "average_paired_scout_lift_percent"
            ]
            output["observations"]["learning"] = [
                {k:row[k] for k in fields if k in row}
                for row in body[:100]
                if isinstance(row, dict)
            ]

        elif endpoint == "feedback-queues" and isinstance(body, dict):
            output["observations"]["queues"] = {
                k:v for k,v in body.items()
                if k.endswith(("_depth", "_age_ms", "_capacity"))
            }
            output["observations"]["delivery_lifecycle"] = body.get(
                "lifecycle", {}
            )

        elif endpoint == "open-position-counts":
            output["observations"]["open_position_counts"] = body

        elif endpoint == "profiles" and isinstance(body, list):
            counts = {}
            for row in body:
                if isinstance(row, dict):
                    strategy = row.get("strategy", "unknown")
                    counts[strategy] = counts.get(strategy, 0)+1
            output["observations"]["profile_counts"] = counts

        elif isinstance(body, dict):
            output["observations"][endpoint+"_fields"] = list(body)[:60]

    output["financial_validation"] = "NOT_VERIFIED"

    if name not in ("trading_health", "feedback_health"):
        output["analysis_status"] = "BLOCKED_PENDING_CONTRACT_MAPPING"
        output["reason"] = (
            "Evidence collected; authoritative financial fields, "
            "mode, units, time scope and population require verification."
        )

    if name == "learning_readiness":
        output["reason"] = (
            "Raw sample counts are not effective independent samples "
            "and do not establish promotion readiness."
        )

    output["report_evidence"] = save(output, name)
    return output

def main(name):
    p = argparse.ArgumentParser(description=name)
    p.parse_args()
    result = report(name)
    emit(result)
    return 1 if (
        result["status"] == "BLOCKED" or
        result.get("analysis_status", "").startswith("BLOCKED")
    ) else 0
