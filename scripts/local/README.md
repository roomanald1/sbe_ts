# Local deterministic toolkit

Read-only trading diagnostics and local implementation support.

## Safety

- No trading mutations, database writes or deployment.
- No automatic scheduler installation.
- Heavy Insights quality endpoints and oversized snapshots excluded.
- Raw evidence stays in ~/.cache/sbe-diagnostics.
- Never commit raw evidence or unreviewed fixtures.
- API filtering does not reduce server-side workload.
- Sample counts are not effective independent samples.
- Financial reports require verified data contracts.
- Static repository analysis is heuristic.
- Tests run only with explicit --execute.
- Mandatory integration tests remain required.
- No automatic strategy promotion or acceptance.

## Usage

python3 scripts/local/trading_health.py
python3 scripts/local/feedback_health.py
python3 scripts/local/learning_readiness.py
python3 scripts/local/integration_check.py
python3 scripts/local/change_impact.py
python3 scripts/local/symbol_index.py --term reconcile
python3 scripts/local/callers.py --term count_auto_trader_decisions
python3 scripts/local/test_runner.py --affected
python3 scripts/local/test_runner.py --crate learning --execute
python3 scripts/local/resource_monitor.py
python3 scripts/local/checkpoint.py

## Implementation status

Trading health and feedback reports collect and project verified
API envelopes. Financial correctness and promotion are not verified.

Other trading reports collect evidence pending financial contract mapping.
Architecture checks require explicit rules.
Fixture capture produces private raw evidence, not sanitised fixtures.
Regression comparisons are structural, not financial acceptance.
Agent budget checks use supplied usage figures.
Work package extraction requires the actual plan path.
Monitoring commands are one-shot; no background services installed.
