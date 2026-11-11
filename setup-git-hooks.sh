#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"

local_hooks_path="$(git config --local --get core.hooksPath || true)"
effective_hooks_path="$(git config --get core.hooksPath || true)"

if [[ "$local_hooks_path" == ".githooks" ]]; then
  printf 'Repository hooks are already configured to use .githooks.\n'
elif [[ -n "$effective_hooks_path" ]]; then
  printf 'Preserving existing Git core.hooksPath setting: %s\n' "$effective_hooks_path"
  printf 'To switch this repository to its own hooks, run: git config --local core.hooksPath .githooks\n'
else
  git config --local core.hooksPath .githooks
  printf 'Configured this repository to use .githooks.\n'
fi
