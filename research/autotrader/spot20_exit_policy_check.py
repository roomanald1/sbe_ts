#!/usr/bin/env python3
"""Screen the 20-day BTC-gated breakout against current fixed brackets."""
import json
from pathlib import Path

from exit_policy_counterfactual import run
from spot_trailing_breakout_screen import load

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "spot20_exit_policy_results.json"


def main():
    dates, data = load()
    results = {
        "date_range": [dates[202], dates[-2]],
        "timing": "prior close signal; next open entry; open-to-next-open holding",
        "portfolio": "fixed 1/7 weight per asset on each active day; rebalancing cost omitted as in legacy exit replay",
        "cost": "15 bps per side",
        "fixed_bracket_assumption": "daily low/high touches; stop first if both touched; gaps at worse open",
        "policies": {
            "signal_exit": run(dates, data, "daily_signal_exit", lookback_days=20),
            "net_stop_0_5_target_0_5": run(dates, data, "fixed_net_brackets", 0.5, 0.5, lookback_days=20),
            "net_stop_0_5_target_1_5": run(dates, data, "fixed_net_brackets", 0.5, 1.5, lookback_days=20),
        },
    }
    OUT.write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({name: result["splits"] for name, result in results["policies"].items()}, indent=2))


if __name__ == "__main__":
    main()
