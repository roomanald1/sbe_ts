#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
docker compose -p sbe-infra -f monitoring/docker-compose.yml stop "$@"
echo "Infrastructure stopped."
