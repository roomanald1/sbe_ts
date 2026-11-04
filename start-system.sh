#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"
docker compose -p sbe-system -f system/compose.yml up -d "$@"
echo "System is running. App: http://localhost:5173"
