#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"

system_compose_args=(-p sbe-system -f system/compose.yml)
if [[ "${SCOUT_WORKERS_ENABLED:-true}" == "true" ]]; then
  system_compose_args+=(--profile scout-workers)
fi

docker compose "${system_compose_args[@]}" up -d --build --force-recreate --remove-orphans

echo "All system services rebuilt and restarted."
