#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR=$(dirname -- "$0")
cd "$SCRIPT_DIR"
PIDFILE=$(pwd)/runtime/source_edge_tester/tester.pid
if [[ ! -f "$PIDFILE" ]]; then
  echo "Source edge tester PID file not found"
  exit 0
fi
PID=$(cat "$PIDFILE")
if kill -0 "$PID" 2>/dev/null; then
  kill -TERM "$PID"
  echo "Sent stop signal to source edge tester (PID $PID)"
else
  echo "Source edge tester is not running"
fi
