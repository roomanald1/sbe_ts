#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR=$(dirname -- "$0")
cd "$SCRIPT_DIR"
RUNTIME=$(pwd)/runtime/source_edge_tester
mkdir -p "$RUNTIME"
PIDFILE=$RUNTIME/tester.pid
if [[ -f "$PIDFILE" ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  echo "Source edge tester already running (PID $(cat "$PIDFILE"))"
  exit 0
fi
nohup python3 "$(pwd)/source_tester/source_edge_tester.py" \
  --output "$RUNTIME/events.jsonl" --duration-seconds 259200 --poll-seconds 30 \
  > "$RUNTIME/tester.log" 2>&1 < /dev/null &
PID=$!
printf "%s\n" "$PID" > "$PIDFILE"
sleep 2
if ! kill -0 "$PID" 2>/dev/null; then
  cat "$RUNTIME/tester.log"
  exit 1
fi
echo "Source edge tester started (PID $PID)"
echo "Events: $RUNTIME/events.jsonl"
echo "Log: $RUNTIME/tester.log"
