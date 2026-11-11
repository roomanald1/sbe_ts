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
    if [[ ! " ${services[*]} " == *" live-signals "* ]]; then
      services+=(live-signals)
    fi
    break
  fi
  if [[ "$service" == "auto-trader" ]]; then
    if [[ ! " ${services[*]} " == *" live-signals "* ]]; then
      services+=(live-signals)
    fi
    break
  fi
done
compose_args=(-p sbe-system -f system/compose.yml)
if [[ "${SCOUT_WORKERS_ENABLED:-true}" == "true" ]]; then
  compose_args+=(--profile scout-workers)
fi
docker compose "${compose_args[@]}" up -d "${services[@]}"
echo "System is running. App: http://localhost:5173"
