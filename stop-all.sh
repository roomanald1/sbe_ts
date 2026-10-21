#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
RUNNER_PID_FILE="$ROOT/logs/start-all.pid"
COMPOSE=(docker compose -f "$ROOT/monitoring/docker-compose.yml")
APP_PORTS=(3001 3002 3003 3004 3005 3006 5173 10001)

is_repo_cwd() {
  local pid="$1" cwd
  [[ -d "/proc/$pid" ]] || return 1
  cwd="$(readlink "/proc/$pid/cwd" 2>/dev/null || true)"
  [[ "$cwd" == "$ROOT" || "$cwd" == "$ROOT"/* ]]
}

is_repo_process() {
  local pid="$1" cmdline
  is_repo_cwd "$pid" || return 1
  cmdline="$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null || true)"
  [[ "$cmdline" == *"nx serve react-app"* || "$cmdline" == *"$ROOT/src/rust/"* || "$cmdline" == *"cargo run --manifest-path src/rust/"* ]]
}

stop_repo_processes() {
  local pid port
  local -a pids=()
  for proc in /proc/[0-9]*; do
    pid="${proc##*/}"
    if is_repo_process "$pid"; then pids+=("$pid"); fi
  done
  for port in "${APP_PORTS[@]}"; do
    while read -r pid; do
      [[ -n "$pid" ]] || continue
      if is_repo_cwd "$pid"; then pids+=("$pid"); fi
    done < <(ss -H -ltnp "sport = :$port" 2>/dev/null | sed -n 's/.*pid=\([0-9][0-9]*\).*/\1/p')
  done
  if ((${#pids[@]})); then
    mapfile -t pids < <(printf '%s\n' "${pids[@]}" | sort -un)
    echo "Stopping this repo's backend and UI processes (PIDs ${pids[*]})."
    kill -TERM "${pids[@]}" 2>/dev/null || true
    for _ in {1..10}; do
      local running=()
      for pid in "${pids[@]}"; do
        if kill -0 "$pid" 2>/dev/null && is_repo_process "$pid"; then running+=("$pid"); fi
      done
      ((${#running[@]})) || break
      sleep 1
    done
    for pid in "${pids[@]}"; do
      if kill -0 "$pid" 2>/dev/null && is_repo_process "$pid"; then kill -KILL "$pid" 2>/dev/null || true; fi
    done
  fi
}

if [[ -f "$RUNNER_PID_FILE" ]]; then
  runner_pid="$(cat "$RUNNER_PID_FILE")"
  runner_cmd="$(tr '\0' ' ' < "/proc/$runner_pid/cmdline" 2>/dev/null || true)"
  if kill -0 "$runner_pid" 2>/dev/null && is_repo_cwd "$runner_pid" && [[ "$runner_cmd" == *"start-all.sh"* ]]; then
    kill -TERM "$runner_pid"
    for _ in {1..10}; do
      kill -0 "$runner_pid" 2>/dev/null || break
      sleep 1
    done
  fi
fi

stop_repo_processes
"${COMPOSE[@]}" down
rm -f "$RUNNER_PID_FILE" "$ROOT/logs/backend.pid" "$ROOT/logs/ui.pid"
echo "Backend, UI, and Grafana stopped."
