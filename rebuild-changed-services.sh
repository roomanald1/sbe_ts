#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"
export SOURCE_REVISION="$(git rev-parse HEAD)"
if [[ -n "$(git status --porcelain)" ]]; then SOURCE_DIRTY=true; else SOURCE_DIRTY=false; fi
export SOURCE_DIRTY
export SOURCE_BUILT_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

system_compose_args=(-p sbe-system -f system/compose.yml)
if [[ "${SCOUT_WORKERS_ENABLED:-true}" == "true" ]]; then
  system_compose_args+=(--profile scout-workers)
fi

git_dir="$(git rev-parse --git-common-dir)"
if [[ "$git_dir" != /* ]]; then git_dir="$ROOT/$git_dir"; fi
state_file="$git_dir/rebuild-changed-services.json"
compose_json="$(docker compose "${system_compose_args[@]}" config --format json)"

if [[ "${1:-}" == "--record-all" ]]; then
  printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py record-all "$state_file"
  echo "Recorded current build inputs as deployed for configured system services."
  exit 0
fi

if (($# > 0)); then
  echo "Usage: $0 [--record-all]" >&2
  exit 2
fi

changed_output="$(printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py changed "$state_file")"
services=()
if [[ -n "$changed_output" ]]; then
  mapfile -t services <<< "$changed_output"
fi

if ((${#services[@]} == 0)); then
  echo "No system service build inputs differ from the last recorded deployment."
  exit 0
fi

printf 'Rebuilding changed services: %s\n' "${services[*]}"
start_timeout="${SYSTEM_START_TIMEOUT_SECONDS:-900}"
if ! [[ "$start_timeout" =~ ^[1-9][0-9]*$ ]]; then
  echo "SYSTEM_START_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 2
fi
build_output="$(printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py build-once "$state_file" "${services[@]}")"
build_services=()
if [[ -n "$build_output" ]]; then
  mapfile -t build_services <<< "$build_output"
fi
if ((${#build_services[@]} > 0)); then
  docker compose --progress quiet "${system_compose_args[@]}" build "${build_services[@]}"
fi
docker compose "${system_compose_args[@]}" up -d --no-build --force-recreate --no-deps --wait --wait-timeout "$start_timeout" "${services[@]}"
printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py record "$state_file" "${services[@]}"
echo "Changed services rebuilt, restarted, and recorded as deployed."
