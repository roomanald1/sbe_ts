#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

# Compare service build inputs with the last successfully deployed fingerprints.
# The helper builds and force-recreates only services whose fingerprints differ.
exec ./rebuild-changed-services.sh "$@"
