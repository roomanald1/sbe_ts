#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"
if [[ ! -f monitoring/.env ]]; then
  echo "Missing monitoring/.env. Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password." >&2
  exit 1
fi
docker compose -p sbe-infra -f monitoring/docker-compose.yml up -d "$@"
echo "Infrastructure is running. Service control: http://localhost:8080"
