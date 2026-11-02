#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "cargo-llvm-cov is required. Install it with cargo install cargo-llvm-cov." >&2
  exit 1
fi
(
  cd src/typescript/workspace
  ./node_modules/.bin/nx run react-app:coverage
)
for crate in auto_trader execution feed_handlers insights position_management; do
  echo "Generating Rust coverage for $crate"
  cargo llvm-cov --manifest-path "src/rust/$crate/Cargo.toml" --html --output-dir "src/rust/$crate/coverage/html"
done
echo "Frontend report: src/typescript/workspace/apps/react-app/coverage/index.html"
for crate in auto_trader execution feed_handlers insights position_management; do
  echo "Rust report: src/rust/$crate/coverage/html/index.html"
done
