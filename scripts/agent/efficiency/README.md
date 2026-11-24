# Agent efficiency utilities

These utilities supplement existing repository tools.
They do not replace RTK, Beads, preparation or regression
infrastructure.

Run from the repository root:

    python3 scripts/agent/efficiency/context_watch.py --role rust

    python3 scripts/agent/efficiency/symbol_lookup.py NAME --crate learning

    python3 scripts/agent/efficiency/bounded_run.py -- just test learning

    python3 scripts/agent/efficiency/tool_catalog.py realized pnl

    python3 scripts/agent/efficiency/navigation_candidates.py

    python3 scripts/agent/efficiency/budget_dashboard.py --since YYYY-MM-DD

    python3 scripts/agent/efficiency/regression_identity.py --command "just test learning"

Context warnings are advisory. Symbol matches and navigation
maps require source verification. Regression fingerprints do
not automatically authorize skipping tests.

Do not automatically write Beads memories from historical
navigation candidates.
