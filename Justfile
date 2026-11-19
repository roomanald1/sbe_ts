# Rebuild and restart selected system services without restarting dependencies.
rebuild +services:
    #!/usr/bin/env bash
    set -euo pipefail
    compose_args=(-p sbe-system -f system/compose.yml --profile scout-workers)
    services=({{services}})
    compose_json="$(docker compose "${compose_args[@]}" config --format json)"
    build_output="$(printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py build-once - "${services[@]}")"
    build_services=()
    if [[ -n "$build_output" ]]; then mapfile -t build_services <<< "$build_output"; fi
    if ((${#build_services[@]} > 0)); then
      docker compose "${compose_args[@]}" build --progress quiet "${build_services[@]}"
    fi
    docker compose "${compose_args[@]}" up -d --no-build --force-recreate --no-deps "${services[@]}"

# Rebuild all 17 Scout worker services.
rebuild-scouts:
    #!/usr/bin/env bash
    set -euo pipefail
    mapfile -t services < <(docker compose -p sbe-system -f system/compose.yml --profile scout-workers config --services | grep '^strategy-scout-')
    if (( ${#services[@]} == 0 )); then
      echo "No Scout worker services found in system/compose.yml" >&2
      exit 1
    fi
    compose_args=(-p sbe-system -f system/compose.yml --profile scout-workers)
    compose_json="$(docker compose "${compose_args[@]}" config --format json)"
    build_output="$(printf '%s' "$compose_json" | python3 rebuild-service-fingerprints.py build-once - "${services[@]}")"
    build_services=()
    if [[ -n "$build_output" ]]; then mapfile -t build_services <<< "$build_output"; fi
    if ((${#build_services[@]} > 0)); then
      docker compose "${compose_args[@]}" build --progress quiet "${build_services[@]}"
    fi
    docker compose "${compose_args[@]}" up -d --no-build --force-recreate --no-deps "${services[@]}"

# Rebuild only the system Compose project.
rebuild-services:
    ./rebuild-services.sh

# Rebuild only services whose build inputs differ from the last recorded deployment.
rebuild-changed-services +args:
    ./rebuild-changed-services.sh {{args}}

# Rebuild infrastructure first, then every system service.
rebuild-all:
    ./rebuild-all.sh

# Format a Rust crate, for example: just fmt strategy.
fmt crate:
    cargo fmt --manifest-path src/rust/{{crate}}/Cargo.toml

# Check a Rust crate, for example: just check auto_trader.
check crate:
    cargo check --manifest-path src/rust/{{crate}}/Cargo.toml

# Test a Rust crate, optionally with Cargo test filters.
test crate *args:
    cargo test --manifest-path src/rust/{{crate}}/Cargo.toml {{args}}

# Format all Rust crates frequently edited in this workspace.
fmt-rust:
    just fmt auto_trader
    just fmt insights
    just fmt learning
    just fmt strategy

# Run the React app TypeScript typecheck.
typecheck:
    ./nx typecheck react-app

# Verify that the current diff has no whitespace errors.
diff-check:
    git diff --check
