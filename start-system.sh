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
docker compose -p sbe-system -f system/compose.yml up -d "${services[@]}"
echo "System is running. App: http://localhost:5173"
