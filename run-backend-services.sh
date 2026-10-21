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
children=()
cleanup() {
  trap - INT TERM EXIT
  for pid in "${children[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${children[@]}"; do wait "$pid" 2>/dev/null || true; done
}
trap cleanup INT TERM EXIT

"$NEWS_SENTIMENT_PYTHON" "$ROOT/src/rust/insights/scripts/classify_news.py" &
sentiment_pid=$!
children+=("$sentiment_pid")
cargo run --manifest-path src/rust/insights/Cargo.toml &
insights_pid=$!
children+=("$insights_pid")
ready=0
for _ in {1..60}; do
  if curl --fail --silent http://127.0.0.1:3004/health >/dev/null; then ready=1; break; fi
  if ! kill -0 "$insights_pid" 2>/dev/null; then wait "$insights_pid"; exit $?; fi
  sleep 1
done
if [[ "$ready" != "1" ]]; then echo "Insights did not become ready on port 3004" >&2; exit 1; fi

cargo run --manifest-path src/rust/feed_handlers/Cargo.toml &
feed_pid=$!
children+=("$feed_pid")
ready=0
for _ in {1..60}; do
  if curl --fail --silent http://127.0.0.1:3001/health >/dev/null; then ready=1; break; fi
  if ! kill -0 "$feed_pid" 2>/dev/null; then wait "$feed_pid"; exit $?; fi
  sleep 1
done
if [[ "$ready" != "1" ]]; then echo "FeedHandlers did not become ready on port 3001" >&2; exit 1; fi


cargo run --manifest-path src/rust/position_management/Cargo.toml &
position_pid=$!
children+=("$position_pid")
ready=0
for _ in {1..60}; do
  if curl --fail --silent http://127.0.0.1:3002/health >/dev/null; then ready=1; break; fi
  if ! kill -0 "$position_pid" 2>/dev/null; then wait "$position_pid"; exit $?; fi
  sleep 1
done
if [[ "$ready" != "1" ]]; then echo "Position Management did not become ready on port 3002" >&2; exit 1; fi

cargo run --manifest-path src/rust/auto_trader/Cargo.toml &
auto_trader_pid=$!
children+=("$auto_trader_pid")
ready=0
for _ in {1..60}; do
  if curl --fail --silent http://127.0.0.1:3005/health >/dev/null; then ready=1; break; fi
  if ! kill -0 "$auto_trader_pid" 2>/dev/null; then wait "$auto_trader_pid"; exit $?; fi
  sleep 1
done
if [[ "$ready" != "1" ]]; then echo "AutoTrader did not become ready on port 3005" >&2; exit 1; fi


cargo run --manifest-path src/rust/execution/Cargo.toml &
children+=("$!")

status=0
wait -n "${children[@]}" || status=$?
exit "$status"
