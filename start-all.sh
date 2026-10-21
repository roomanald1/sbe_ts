#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
LOG_DIR="$ROOT/logs"
RUNNER_PID_FILE="$LOG_DIR/start-all.pid"
BACKEND_PID_FILE="$LOG_DIR/backend.pid"
UI_PID_FILE="$LOG_DIR/ui.pid"
COMPOSE=(docker compose -f "$ROOT/monitoring/docker-compose.yml")

if [[ ! -f "$ROOT/monitoring/.env" ]]; then
  echo "Missing monitoring/.env. Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password." >&2
  exit 1
fi
mkdir -p "$LOG_DIR"
./stop-all.sh
echo "$$" > "$RUNNER_PID_FILE"
children=()
cleanup() {
  status=$?
  trap - INT TERM EXIT
  for pid in "${children[@]}"; do kill -TERM "$pid" 2>/dev/null || true; done
  for pid in "${children[@]}"; do wait "$pid" 2>/dev/null || true; done
  "${COMPOSE[@]}" down || true
  rm -f "$RUNNER_PID_FILE" "$BACKEND_PID_FILE" "$UI_PID_FILE"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

"${COMPOSE[@]}" up -d --build

./run-backend-services.sh >"$LOG_DIR/backend.log" 2>&1 &
backend_pid=$!
children+=("$backend_pid")
echo "$backend_pid" > "$BACKEND_PID_FILE"

./nx serve react-app >"$LOG_DIR/ui.log" 2>&1 &
ui_pid=$!
children+=("$ui_pid")
echo "$ui_pid" > "$UI_PID_FILE"

echo "App:       http://localhost:5173"
echo "Grafana:   http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading"
echo "Prometheus: http://localhost:9090"
echo "Press Ctrl+C here, or run ./stop-all.sh, to stop the stack."

wait -n "$backend_pid" "$ui_pid"
