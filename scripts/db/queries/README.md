# Verified SQL catalogue

Create one JSON file per verified recurring query.

Required structure:

{
  "description": "What the query measures",
  "database": "learning",
  "file": "verified_database_filename.sqlite",
  "sql": "Verified parameterised SELECT statement",
  "required_parameters": ["since_ms"],
  "verified": false,
  "verification": "",
  "units": {},
  "scope": "Explicit mode, epoch and time scope"
}

Do not set verified=true until:

- Actual database schema has been inspected.
- Query execution plan has been checked.
- Time, currency, mode and profile semantics are confirmed.
- Results agree with authoritative application data.
- Verification evidence has been recorded.

Do not invent tables or financial field definitions.
