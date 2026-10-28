#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

node_version_supported() {
  node -e 'const [major, minor] = process.versions.node.split(".").map(Number); process.exit((major === 20 && minor >= 19) || (major >= 22 && (major > 22 || minor >= 12)) ? 0 : 1)' >/dev/null 2>&1
}
if ! node_version_supported; then
  NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
  if [[ -s "$NVM_DIR/nvm.sh" ]]; then
    . "$NVM_DIR/nvm.sh"
    nvm use 22 >/dev/null || true
  fi
fi
if ! node_version_supported; then
  echo "React app requires Node.js 20.19+ or 22.12+; found $(node --version 2>/dev/null || echo unavailable)." >&2
  exit 1
fi

process_alive() {
  local pid="$1" state
  [[ -r "/proc/$pid/stat" ]] || return 1
  state="$(awk '{print $3}' "/proc/$pid/stat")"
  [[ "$state" != "Z" && "$state" != "X" ]]
}

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

if [[ "${1:-}" != "--daemon-run" ]]; then
  if [[ -f "$RUNNER_PID_FILE" ]]; then
    runner_pid="$(cat "$RUNNER_PID_FILE")"
    if process_alive "$runner_pid"; then
      echo "Stack supervisor is already running (PID $runner_pid)."
      exit 0
    fi
  fi
  ./stop-all.sh
  nohup "$0" --daemon-run >>"$LOG_DIR/start-all.log" 2>&1 </dev/null &
  runner_pid=$!
  echo "$runner_pid" > "$RUNNER_PID_FILE"
  disown "$runner_pid" 2>/dev/null || true
  echo "Stack startup launched in the background (PID $runner_pid)."
  echo "Logs: $LOG_DIR/"
  exit 0
fi

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
ui_ready=0
for _ in {1..30}; do
  if curl --fail --silent http://127.0.0.1:5173/ >/dev/null; then ui_ready=1; break; fi
  if ! process_alive "$ui_pid"; then echo "React app exited before becoming ready; see logs/ui.log" >&2; exit 1; fi
  sleep 1
done
if [[ "$ui_ready" != "1" ]]; then echo "React app did not become ready on port 5173 within 30 seconds; see logs/ui.log" >&2; exit 1; fi

echo "App:       http://localhost:5173"
echo "Grafana:   http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading"
echo "Prometheus: http://localhost:9090"
echo "Run ./stop-all.sh to stop the stack."

wait -n "$backend_pid" "$ui_pid"
