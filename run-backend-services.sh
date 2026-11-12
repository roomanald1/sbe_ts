#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export POSITIONS_FILE="$ROOT/src/rust/position_management/positions.json"
export AUTO_TRADER_STATE_FILE="$ROOT/src/rust/auto_trader/auto_trader_state.json"
export SIGNALS_FILE="$ROOT/src/rust/insights/signals.jsonl"
export NEWS_SENTIMENT_PYTHON="${NEWS_SENTIMENT_PYTHON:-python3}"
export PYTHONPATH="$ROOT/src/rust/insights/.news-python-packages${PYTHONPATH:+:$PYTHONPATH}"
export HF_HOME="${HF_HOME:-$ROOT/src/rust/insights/.news-model-cache}"
export NEWS_SENTIMENT_URL="${NEWS_SENTIMENT_URL:-http://127.0.0.1:3006/classify}"
export LEARNING_SERVICE_URL="${LEARNING_SERVICE_URL:-http://127.0.0.1:3010}"
export LEARNING_STATE_FILE="${LEARNING_STATE_FILE:-$ROOT/src/rust/learning/learning_state.sqlite}"
export LEARNING_RSOCKET_URL="ws://127.0.0.1:10010"
export INSIGHTS_RPC_RSOCKET_URL="ws://127.0.0.1:10007"
export AUTO_TRADER_RSOCKET_URL="ws://127.0.0.1:10008"
export POSITION_MANAGEMENT_RSOCKET_URL="ws://127.0.0.1:10005"
export FEED_HANDLERS_RSOCKET_URL="ws://127.0.0.1:10001"
export BINANCE_FEED_RSOCKET_URL="ws://127.0.0.1:10002"
export NEWS_SENTIMENT_RSOCKET_URL="ws://127.0.0.1:10012/rsocket"
export FEED_HANDLERS_RSOCKET_PORT=10001
export BINANCE_FEED_RSOCKET_PORT=10002
export POSITION_MANAGEMENT_RSOCKET_PORT=10005
export EXECUTION_RSOCKET_PORT=10006
export INSIGHTS_RSOCKET_PORT=10004
export INSIGHTS_RPC_RSOCKET_PORT=10007
export INSIGHTS_BROWSER_RSOCKET_PORT=10011
export LEARNING_RSOCKET_PORT=10010
export AUTO_TRADER_RSOCKET_PORT=10008

strategies=(
  live-momentum
  live-trade-flow
  live-order-book
  live-vwap-dislocation
  live-absorption-pressure
  donchian-55-btc200
  donchian-20-sma200
  donchian-20-btc200
  donchian-10-btc200
  donchian-20-btc200-hold3
  risk-adjusted-trend-portfolio
  trend-sleeves-btc200
  trend-50-200
  relative-strength-90-sma200
  risk-adjusted-monthly-momentum
  rsi2-oversold-rebound
  dated-futures-carry
)

live_endpoints=""
scout_endpoints=""
for index in "${!strategies[@]}"; do
  strategy="${strategies[$index]}"
  live_port=$((12001 + index))
  scout_port=$((13001 + index))
  live_endpoint="${strategy}=ws://127.0.0.1:${live_port}"
  scout_endpoint="${strategy}=ws://127.0.0.1:${scout_port}"
  if [[ -n "$live_endpoints" ]]; then
    live_endpoints+=","$live_endpoint
    scout_endpoints+=";"$scout_endpoint
  else
    live_endpoints="$live_endpoint"
    scout_endpoints="$scout_endpoint"
  fi
done
export AUTO_TRADER_LIVE_WORKER_ENDPOINTS="$live_endpoints"
export SCOUT_WORKER_ENDPOINTS="$scout_endpoints"

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

wait_for_worker() {
  local pid="$1" metrics_port="$2" name="$3" status
  for _ in {1..60}; do
    if curl --fail --silent "http://127.0.0.1:$metrics_port/health" >/dev/null; then return 0; fi
    if ! process_alive "$pid"; then
      if wait "$pid"; then status=0; else status=$?; fi
      echo "$name exited before its metrics listener became healthy (status $status)" >&2
      (( status != 0 )) || status=1
      return "$status"
    fi
    sleep 1
  done
  echo "$name did not become ready within 60 seconds" >&2
  return 1
}

start_strategy_worker() {
  local role="$1" strategy="$2" index="$3" port metrics_port state_path pid
  if [[ "$role" == "live" ]]; then
    port=$((12001 + index))
    metrics_port=$((14001 + index))
    STRATEGY_X_WORKER_ROLE=live \
    STRATEGY_X_WORKER_STRATEGY="$strategy" \
    STRATEGY_X_LIVE_LISTEN_ADDR="127.0.0.1:$port" \
    STRATEGY_X_METRICS_LISTEN_ADDR="127.0.0.1:$metrics_port" \
      cargo run --release --manifest-path src/rust/strategy/Cargo.toml --bin strategy --features service &
  else
    port=$((13001 + index))
    metrics_port=$((15001 + index))
    state_path="$ROOT/src/rust/learning/strategy_state/$strategy.sqlite"
    STRATEGY_X_WORKER_ROLE=scout \
    STRATEGY_X_WORKER_STRATEGY="$strategy" \
    SCOUT_WORKER_LISTEN_ADDR="127.0.0.1:$port" \
    SCOUT_WORKER_STATE_PATH="$state_path" \
    STRATEGY_X_METRICS_LISTEN_ADDR="127.0.0.1:$metrics_port" \
      cargo run --release --manifest-path src/rust/strategy/Cargo.toml --bin strategy --features service &
  fi
  pid=$!
  children+=("$pid")
  wait_for_worker "$pid" "$metrics_port" "Strategy X $role worker $strategy"
}

cargo run --release --manifest-path src/rust/learning/Cargo.toml --bin learning &
learning_pid=$!
children+=("$learning_pid")
wait_for_service "$learning_pid" 3010 "Learning"

"$NEWS_SENTIMENT_PYTHON" "$ROOT/src/rust/insights/scripts/classify_news.py" &
sentiment_pid=$!
children+=("$sentiment_pid")

cargo run --release --manifest-path src/rust/feed_handlers/Cargo.toml &
feed_pid=$!
children+=("$feed_pid")
wait_for_service "$feed_pid" 3001 "FeedHandlers"

cargo run --release --manifest-path src/rust/binance_feed/Cargo.toml &
binance_pid=$!
children+=("$binance_pid")
wait_for_service "$binance_pid" 3012 "Binance Feed"

cargo run --release --manifest-path src/rust/position_management/Cargo.toml &
position_pid=$!
children+=("$position_pid")
wait_for_service "$position_pid" 3002 "Position Management"

cargo run --release --manifest-path src/rust/execution/Cargo.toml &
execution_pid=$!
children+=("$execution_pid")
wait_for_service "$execution_pid" 3003 "Execution"

cargo run --release --manifest-path src/rust/insights/Cargo.toml --bin insights &
insights_pid=$!
children+=("$insights_pid")
wait_for_service "$insights_pid" 3004 "Insights"
wait_for_service "$sentiment_pid" 3006 "News sentiment"

cargo run --release --manifest-path src/rust/insights/Cargo.toml --bin insights-maintenance &
insights_maintenance_pid=$!
children+=("$insights_maintenance_pid")

mkdir -p "$ROOT/src/rust/learning/strategy_state"
for index in "${!strategies[@]}"; do
  start_strategy_worker live "${strategies[$index]}" "$index"
done

cargo run --release --manifest-path src/rust/auto_trader/Cargo.toml --bin auto_trader &
auto_trader_pid=$!
children+=("$auto_trader_pid")
wait_for_service "$auto_trader_pid" 3005 "AutoTrader"

for index in "${!strategies[@]}"; do
  start_strategy_worker scout "${strategies[$index]}" "$index"
done

status=0
wait -n "${children[@]}" || status=$?
exit "$status"
