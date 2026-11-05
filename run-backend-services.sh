#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export POSITIONS_FILE="$ROOT/src/rust/position_management/positions.json"
export AUTO_TRADER_STATE_FILE="$ROOT/src/rust/position_management/auto_trader_state.json"
export SIGNALS_FILE="$ROOT/src/rust/insights/signals.jsonl"
export NEWS_SENTIMENT_PYTHON="${NEWS_SENTIMENT_PYTHON:-python3}"
export PYTHONPATH="$ROOT/src/rust/insights/.news-python-packages${PYTHONPATH:+:$PYTHONPATH}"
export HF_HOME="${HF_HOME:-$ROOT/src/rust/insights/.news-model-cache}"
export NEWS_SENTIMENT_URL="${NEWS_SENTIMENT_URL:-http://127.0.0.1:3006/classify}"
export LEARNING_SERVICE_URL="${LEARNING_SERVICE_URL:-http://127.0.0.1:3010}"
export SCOUTS_SERVICE_URL="${SCOUTS_SERVICE_URL:-http://127.0.0.1:3011}"
export LEARNING_STATE_FILE="${LEARNING_STATE_FILE:-$ROOT/src/rust/position_management/learning_state.json}"
children=()
cleanup() {
  trap - INT TERM EXIT
  for pid in "${children[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${children[@]}"; do wait "$pid" 2>/dev/null || true; done
}
trap cleanup INT TERM EXIT

process_alive() {
  local pid="$1" state
  [[ -r "/proc/$pid/stat" ]] || return 1
  state="$(awk '{print $3}' "/proc/$pid/stat")"
  [[ "$state" != "Z" && "$state" != "X" ]]
}

wait_for_service() {
  local pid="$1" port="$2" name="$3" status
  for _ in {1..60}; do
    if curl --fail --silent "http://127.0.0.1:$port/health" >/dev/null; then return 0; fi
    if ! process_alive "$pid"; then
      if wait "$pid"; then status=0; else status=$?; fi
      echo "$name exited before becoming healthy on port $port (status $status)" >&2
      (( status != 0 )) || status=1
      return "$status"
    fi
    sleep 1
  done
  echo "$name did not become ready on port $port within 60 seconds" >&2
  return 1
}

cargo run --release --manifest-path src/rust/learning/Cargo.toml --bin learning &
learning_pid=$!
children+=("$learning_pid")
wait_for_service "$learning_pid" 3010 "Learning"

cargo run --release --manifest-path src/rust/scouts/Cargo.toml --bin scouts &
scouts_pid=$!
children+=("$scouts_pid")
wait_for_service "$scouts_pid" 3011 "Scouts"

"$NEWS_SENTIMENT_PYTHON" "$ROOT/src/rust/insights/scripts/classify_news.py" &
sentiment_pid=$!
children+=("$sentiment_pid")
cargo run --release --manifest-path src/rust/insights/Cargo.toml --bin insights &
insights_pid=$!
children+=("$insights_pid")
wait_for_service "$insights_pid" 3004 "Insights"
wait_for_service "$sentiment_pid" 3006 "News sentiment"

cargo run --release --manifest-path src/rust/insights/Cargo.toml --bin insights-maintenance &
insights_maintenance_pid=$!
children+=("$insights_maintenance_pid")

cargo run --release --manifest-path src/rust/feed_handlers/Cargo.toml &
feed_pid=$!
children+=("$feed_pid")
wait_for_service "$feed_pid" 3001 "FeedHandlers"

cargo run --release --manifest-path src/rust/position_management/Cargo.toml &
position_pid=$!
children+=("$position_pid")
wait_for_service "$position_pid" 3002 "Position Management"

cargo run --release --manifest-path src/rust/auto_trader/Cargo.toml &
auto_trader_pid=$!
children+=("$auto_trader_pid")
wait_for_service "$auto_trader_pid" 3005 "AutoTrader"

cargo run --release --manifest-path src/rust/execution/Cargo.toml &
execution_pid=$!
children+=("$execution_pid")
wait_for_service "$execution_pid" 3003 "Execution"

status=0
wait -n "${children[@]}" || status=$?
exit "$status"
