#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
if [[ ! -f monitoring/.env ]]; then
  echo "Missing monitoring/.env. Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password." >&2
  exit 1
fi
./start-system.sh
./start-infra.sh
echo "Service control: http://localhost:8080"
echo "App:            http://localhost:5173"
echo "Coverage:       http://localhost:8080/coverage/"
echo "Grafana:        http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading"
echo "Prometheus:     http://localhost:9090"
