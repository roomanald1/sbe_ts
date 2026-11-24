# Deterministic diagnostic tooling

## Rules

- Prefer existing diagnostic scripts to repeated manual investigation.
- All tools are read-only with respect to trading systems.
- Do not install dependencies unnecessarily.
- Keep normal output compact.
- Inspect detailed JSON evidence only when required.
- Never invent database schemas, endpoints or field mappings.
- Use existing authoritative Rust calculations and service contracts.
- Do not treat descriptive statistics as validated trading evidence.
- Do not activate strategies based on these tools alone.
- Preserve mode, currency, profile, epoch and time scope.
- Missing data is a blocker, not a zero-valued result.

## Immediately usable tools

    python3 scripts/agent/agent_usage.py --hours 24
    python3 scripts/agent/token_trends.py --hours 168
    python3 scripts/agent/context_audit.py --hours 48
    python3 scripts/agent/repo_index.py autotrader
    python3 scripts/agent/repo_index.py SignalSnapshot --symbols
    python3 scripts/diagnostics/system_health.py --stats
    python3 scripts/diagnostics/test_summary.py /tmp/test.log
    python3 scripts/diagnostics/sqlite_inspect.py DATABASE

## Trading diagnostic input

Trading diagnostics accept CSV, JSON arrays, JSON objects containing
a rows array, or JSONL.

They require explicit verified field mappings. They do not automatically
discover or query the production trading database.

Run each script with --help for command-line arguments.
Inspect require() calls for required input fields.

## Phase 0 integration

1. Discover existing diagnostics before adding new ones.
2. Reuse existing authoritative APIs, exports and Rust calculations.
3. Implement bounded read-only data adapters.
4. Document source, units, scope and field mappings.
5. Validate against known fixtures and existing authoritative totals.
6. Add regression tests for missing, duplicate and malformed data.
7. Record verified adapter locations with bd remember.
8. Do not duplicate existing financial or statistical engines.

These scripts are analysis helpers, not substitutes for v31 acceptance
gates or independent financial correctness review.
