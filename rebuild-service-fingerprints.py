#!/usr/bin/env python3
"""Select and record system services based on their current build inputs."""

import hashlib
import json
import os
import subprocess
import sys
import tomllib


def crate_for(service):
    build = service.get("build") or {}
    return (build.get("args") or {}).get("CRATE")


def dependency_paths(crate):
    """Return a crate and its recursive local Cargo path dependencies."""
    rust_root = os.path.realpath("src/rust")
    pending = [os.path.join(rust_root, crate)]
    found = set()
    while pending:
        package_dir = os.path.realpath(pending.pop())
        if package_dir in found:
            continue
        found.add(package_dir)
        manifest_path = os.path.join(package_dir, "Cargo.toml")
        if not os.path.isfile(manifest_path):
            continue
        with open(manifest_path, "rb") as manifest_file:
            manifest = tomllib.loads(manifest_file.read().decode("utf-8-sig"))

        tables = [
            manifest.get(name, {})
            for name in ("dependencies", "dev-dependencies", "build-dependencies")
        ]
        tables.extend(
            target.get(name, {})
            for target in manifest.get("target", {}).values()
            for name in ("dependencies", "dev-dependencies", "build-dependencies")
        )
        tables.extend(manifest.get("patch", {}).values())
        for table in tables:
            for dependency in table.values():
                if isinstance(dependency, dict) and dependency.get("path"):
                    path = os.path.realpath(os.path.join(package_dir, dependency["path"]))
                    if os.path.commonpath([rust_root, path]) == rust_root:
                        pending.append(path)
    return sorted(found)


def digest_paths(paths):
    names = set()
    for path in paths:
        path = os.path.relpath(os.path.abspath(path), os.getcwd())
        tracked = subprocess.check_output(
            ["git", "ls-files", "-z", "--", path]
        ).decode().split("\0")
        untracked = subprocess.check_output(
            ["git", "ls-files", "--others", "--exclude-standard", "-z", "--", path]
        ).decode().split("\0")
        names.update(name for name in tracked + untracked if name and os.path.isfile(name))

    digest = hashlib.sha256()
    for name in sorted(names):
        blob = subprocess.check_output(["git", "hash-object", "--", name]).decode().strip()
        digest.update(name.encode() + b"\0" + blob.encode() + b"\0")
    return digest.hexdigest()


def fingerprint(service):
    build = service.get("build") or {}
    if not build:
        return None

    context = build.get("context", ".")
    crate = crate_for(service)
    paths = ["system/compose.yml", ".dockerignore"]
    paths.extend(dependency_paths(crate) if crate else [context])
    dockerfile = build.get("dockerfile", "Dockerfile")
    paths.append(dockerfile if os.path.isabs(dockerfile) else os.path.join(context, dockerfile))

    source_digest = digest_paths(paths)
    config_digest = hashlib.sha256(
        json.dumps(service, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    return f"v2:{source_digest}:{config_digest}"


def load_state(path):
    try:
        with open(path) as state_file:
            return json.load(state_file)
    except FileNotFoundError:
        return {}


def save_state(path, state):
    with open(path, "w") as state_file:
        json.dump(state, state_file, indent=2, sort_keys=True)
        state_file.write("\n")


def main():
    mode, state_path, *selected_names = sys.argv[1:]
    config = json.load(sys.stdin)
    services = config.get("services", {})

    if mode == "changed":
        state = load_state(state_path)
        for name, service in services.items():
            current = fingerprint(service)
            if current and current != state.get(name):
                print(name)
        return

    if mode == "build-once":
        seen = set()
        for name in selected_names:
            service = services.get(name, {})
            build = service.get("build") or {}
            if not build:
                continue
            image = service.get("image") or f"service:{name}"
            key = (image, json.dumps(build, sort_keys=True, separators=(",", ":")))
            if key not in seen:
                print(name)
                seen.add(key)
        return

    state = load_state(state_path)
    selected = services if mode == "record-all" else selected_names
    for name in selected:
        service = services.get(name, {})
        current = fingerprint(service)
        if current:
            state[name] = current
    save_state(state_path, state)


if __name__ == "__main__":
    main()
