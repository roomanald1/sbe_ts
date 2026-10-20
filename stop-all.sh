#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
RUNNER_PID_FILE="$ROOT/logs/start-all.pid"
COMPOSE=(docker compose -f "$ROOT/monitoring/docker-compose.yml")

if [[ -f "$RUNNER_PID_FILE" ]]; then
  runner_pid="$(cat "$RUNNER_PID_FILE")"
  if kill -0 "$runner_pid" 2>/dev/null; then
    kill -TERM "$runner_pid"
    for _ in {1..30}; do
      kill -0 "$runner_pid" 2>/dev/null || break
      sleep 1
    done
    if kill -0 "$runner_pid" 2>/dev/null; then
      echo "The app supervisor is still stopping; check logs/backend.log and logs/ui.log." >&2
      exit 1
    fi
  else
    rm -f "$RUNNER_PID_FILE"
    "${COMPOSE[@]}" down
  fi
else
  "${COMPOSE[@]}" down
fi

echo "Backend, UI, and Grafana stopped."
