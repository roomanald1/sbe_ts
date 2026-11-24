# SQLite diagnostic tooling

Use these tools before manually rediscovering database schemas.

## Database discovery

    python3 scripts/db/db_discover.py
    python3 scripts/db/db_health.py

## Schema inspection

    python3 scripts/db/db_schema.py learning
    python3 scripts/db/db_find.py trade_id
    python3 scripts/db/db_find.py profile_id

Specify --file when a Docker volume contains multiple databases.

## Read-only queries

    python3 scripts/db/db_query.py learning \
      --sql "SELECT name FROM sqlite_master WHERE type='table'"

Use --param NAME=VALUE for parameterised SQL.
Use --output /tmp/unique-name.json for detailed evidence.

## Safety rules

- Prefer authoritative service APIs for financial calculations.
- Never substitute local databases for Docker databases.
- Verify the active database filename against service configuration.
- Use short indexed queries with bounded time ranges.
- Inspect query plans before expensive queries.
- Do not run unbounded COUNT, GROUP BY, ORDER BY or integrity scans.
- Never write, migrate, vacuum or create indexes through these tools.
- Do not copy large active SQLite files.
- Do not use immutable mode against changing WAL databases.
- Treat truncated results as incomplete evidence.
- Avoid BLOB and large JSON columns unless essential.
- Store recurring verified queries in queries/.
- Record verified database mappings using bd remember.

The default query timeout is five seconds.
The default query output limit is twenty rows.
Large inline results are redirected to temporary evidence files.

Docker access requires Python inside the selected existing container.
Unavailable access must be reported, not silently bypassed.

These tools are diagnostic helpers, not a financial correctness gate.
