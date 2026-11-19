#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"

if [[ ! -f monitoring/.env ]]; then
  echo "Missing monitoring/.env. Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password." >&2
  exit 1
fi

./nx build homepage
./nx build benchmarks

system_compose_args=(-p sbe-system -f system/compose.yml)
if [[ "${SCOUT_WORKERS_ENABLED:-true}" == "true" ]]; then
  system_compose_args+=(--profile scout-workers)
fi

docker compose -p sbe-infra -f monitoring/docker-compose.yml up -d --build --force-recreate --remove-orphans
compose_json="$(docker compose "${system_compose_args[@]}" config --format json)"
services_output="$(printf '%s' "$compose_json" | python3 -c 'import json,sys; print("\n".join(json.load(sys.stdin).get("services", {})))')"
services=()
if [[ -n "$services_output" ]]; then mapfile -t services <<< "$services_output"; fi
build_output="$(printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py build-once - "${services[@]}")"
build_services=()
if [[ -n "$build_output" ]]; then mapfile -t build_services <<< "$build_output"; fi
if ((${#build_services[@]} > 0)); then
  docker compose "${system_compose_args[@]}" build --progress quiet "${build_services[@]}"
fi
docker compose "${system_compose_args[@]}" up -d --no-build --force-recreate --remove-orphans
./rebuild-changed-services.sh --record-all

echo "All system and infrastructure services rebuilt and restarted."
echo "Service control: http://localhost:8080"
echo "App:            http://localhost:5173"
