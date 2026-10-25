#!/usr/bin/env bash
set -euo pipefail
PID_FILE="/home/ronnie/dev/sbe_ts/research/autotrader/runtime/cross_asset_paper/supervisor.pid"
if [[ ! -r "$PID_FILE" ]]; then
  echo "No active paper-run PID file found" >&2
  exit 1
fi
pid="$(cat "$PID_FILE")"
kill -TERM "$pid"
echo "Stopping paper run (supervisor PID $pid); child services will shut down cleanly."
