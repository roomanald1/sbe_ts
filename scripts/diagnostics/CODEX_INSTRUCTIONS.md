# Diagnostic tooling policy

Before performing repeated repository or runtime investigations:

1. Search bd memories for verified component knowledge.
2. Check scripts/agent and scripts/diagnostics for existing tooling.
3. Reuse deterministic scripts rather than reconstructing equivalent
   queries, calculations or log summaries.
4. Keep large outputs in /tmp and read only relevant evidence.
5. Use RTK for supported commands.
6. Extend existing diagnostic scripts when recurring analysis is missing.
7. Validate data adapters against authoritative service contracts.
8. Never invent financial fields or silently substitute missing data.
9. Use model reasoning for interpretation, design and correctness,
   not repetitive extraction and arithmetic.
10. Record verified script usage and data mappings in Beads.

During Phase 0, audit these scripts, reuse equivalent existing tooling,
connect verified data adapters and add regression tests.
Do not mark any trading acceptance gate complete merely because
a diagnostic script executes successfully.
