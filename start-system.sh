#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"
services=("$@")
for service in "${services[@]}"; do
  if [[ "$service" == "insights" ]]; then
    if [[ ! " ${services[*]} " == *" insights-maintenance "* ]]; then
      services+=(insights-maintenance)
    fi
    break
  fi
  if [[ "$service" == "auto-trader" ]]; then
    break
  fi
done
compose_args=(-p sbe-system -f system/compose.yml)
start_timeout="${SYSTEM_START_TIMEOUT_SECONDS:-900}"
if ! [[ "$start_timeout" =~ ^[1-9][0-9]*$ ]]; then
  echo "SYSTEM_START_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 2
fi
if [[ "${SCOUT_WORKERS_ENABLED:-true}" == "true" ]]; then
  compose_args+=(--profile scout-workers)
  if [[ " ${services[*]} " == *" learning "* ]]; then
    while IFS= read -r worker; do
      services+=("$worker")
    done < <(docker compose "${compose_args[@]}" config --services | rg '^strategy-scout-')
  fi
fi
if [[ ${#services[@]} -eq 0 ]]; then
  docker compose "${compose_args[@]}" up -d --wait --wait-timeout "$start_timeout" --remove-orphans
else
  docker compose "${compose_args[@]}" up -d --wait --wait-timeout "$start_timeout" "${services[@]}"
fi
echo "System services are healthy. App: http://localhost:5173"
