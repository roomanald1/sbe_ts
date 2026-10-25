#!/usr/bin/env bash
set -euo pipefail
ROOT="/home/ronnie/dev/sbe_ts"
RUNTIME="$ROOT/research/autotrader/runtime/cross_asset_paper"
INSIGHTS="$ROOT/src/rust/insights/target/debug/insights"
WATCHER="$ROOT/src/rust/insights/target/debug/cross_asset_paper_test"
FEED="$ROOT/src/rust/feed_handlers/target/debug/feed_handlers"
DURATION_SECONDS="${CROSS_ASSET_DURATION_SECONDS:-259200}"
mkdir -p "$RUNTIME"
echo "$$" > "$RUNTIME/supervisor.pid"

if curl --noproxy "*" --max-time 2 --fail --silent http://127.0.0.1:3001/health >/dev/null || curl --noproxy "*" --max-time 2 --fail --silent http://127.0.0.1:3004/health >/dev/null; then
  echo "Port 3001 or 3004 is already serving; refusing to attach to or stop another stack." >&2
  exit 1
fi

children=()
cleanup() {
  trap - EXIT INT TERM
  for pid in "${children[@]}"; do kill -TERM "$pid" 2>/dev/null || true; done
  for pid in "${children[@]}"; do wait "$pid" 2>/dev/null || true; done
  rm -f "$RUNTIME/supervisor.pid"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cd "$ROOT"
cargo build --manifest-path src/rust/insights/Cargo.toml --bin insights --bin cross_asset_paper_test
cargo build --manifest-path src/rust/feed_handlers/Cargo.toml --bin feed_handlers

SIGNALS_DB_FILE="$RUNTIME/signals.sqlite" "$INSIGHTS" >"$RUNTIME/insights.log" 2>&1 &
insights_pid=$!
children+=("$insights_pid")
for _ in $(seq 1 120); do
  if curl --noproxy "*" --max-time 2 --fail --silent http://127.0.0.1:3004/health >/dev/null; then break; fi
  if ! kill -0 "$insights_pid" 2>/dev/null; then cat "$RUNTIME/insights.log" >&2; exit 1; fi
  sleep 1
done
curl --noproxy "*" --max-time 2 --fail --silent http://127.0.0.1:3004/health >/dev/null || { echo "Insights health check timed out" >&2; exit 1; }

"$FEED" >"$RUNTIME/feed_handlers.log" 2>&1 &
feed_pid=$!
children+=("$feed_pid")
for _ in $(seq 1 120); do
  if curl --noproxy "*" --max-time 2 --fail --silent http://127.0.0.1:3001/health >/dev/null; then break; fi
  if ! kill -0 "$feed_pid" 2>/dev/null; then cat "$RUNTIME/feed_handlers.log" >&2; exit 1; fi
  sleep 1
done
curl --noproxy "*" --max-time 2 --fail --silent http://127.0.0.1:3001/health >/dev/null || { echo "FeedHandlers health check timed out" >&2; exit 1; }

"$WATCHER" "$RUNTIME/signals.sqlite" "$RUNTIME/events.jsonl" "$DURATION_SECONDS" 2 >"$RUNTIME/watcher.log" 2>&1 &
watcher_pid=$!
children+=("$watcher_pid")
echo "PAPER_RUN_STARTED duration_seconds=$DURATION_SECONDS output=$RUNTIME/events.jsonl"

status=0
wait -n -p exited_pid "$watcher_pid" "$insights_pid" "$feed_pid" || status=$?
if [[ "${exited_pid:-}" == "$watcher_pid" && "$status" -eq 0 ]]; then
  echo "PAPER_RUN_FINISHED output=$RUNTIME/events.jsonl"
else
  echo "Paper run stopped early (process=${exited_pid:-unknown}, status=$status)" >&2
  cat "$RUNTIME/watcher.log" "$RUNTIME/feed_handlers.log" "$RUNTIME/insights.log" >&2
  exit 1
fi
