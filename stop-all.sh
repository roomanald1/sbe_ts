#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
./stop-system.sh
./stop-infra.sh
echo "System and infrastructure services stopped."
