#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"
coverage_dir="$repo_root/src/typescript/workspace/apps/react-app/public/coverage"
if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "cargo-llvm-cov is required. Install it with cargo install cargo-llvm-cov." >&2
  exit 1
fi
(
  cd src/typescript/workspace
  ./node_modules/.bin/nx run react-app:coverage
)
rust_failed=0
for crate in auto_trader execution feed_handlers insights position_management; do
  echo "Generating Rust coverage for $crate"
  if ! cargo llvm-cov --manifest-path "src/rust/$crate/Cargo.toml" --html --output-dir "$coverage_dir/rust/$crate"; then
    echo "Rust coverage failed for $crate" >&2
    rust_failed=1
  fi
done
cat > "$coverage_dir/index.html" <<'HTML'
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Combined code coverage</title>
<style>body{font:16px system-ui,sans-serif;margin:2rem auto;max-width:900px;padding:0 1rem;color:#20242a}h1{margin-bottom:.25rem}p{color:#59636e}ul{list-style:none;padding:0;display:grid;grid-template-columns:repeat(auto-fit,minmax(230px,1fr));gap:1rem}li{border:1px solid #d9dee5;border-radius:8px;padding:1rem}a{font-weight:650;color:#1769aa}</style></head>
<body><h1>Combined code coverage</h1><p>Coverage reports for the frontend and Rust services.</p><h2>TypeScript</h2><ul><li><a href="typescript/index.html">React app coverage report</a></li></ul><h2>Rust</h2>
<ul><li><a href="rust/auto_trader/html/index.html">auto_trader</a></li><li><a href="rust/execution/html/index.html">execution</a></li><li>feed_handlers report unavailable: synthetic feed test failed</li><li><a href="rust/insights/html/index.html">insights</a></li><li><a href="rust/position_management/html/index.html">position_management</a></li></ul></body></html>
HTML
echo "Combined report: $coverage_dir/index.html"
if [[ "$rust_failed" -ne 0 ]]; then exit 1; fi
