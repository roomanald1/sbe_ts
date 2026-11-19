#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
export REPO_ROOT="$ROOT"

system_compose_args=(-p sbe-system -f system/compose.yml)
if [[ "${SCOUT_WORKERS_ENABLED:-true}" == "true" ]]; then
  system_compose_args+=(--profile scout-workers)
fi

git_dir="$(git rev-parse --git-common-dir)"
if [[ "$git_dir" != /* ]]; then git_dir="$ROOT/$git_dir"; fi
state_file="$git_dir/rebuild-changed-services.json"
compose_json="$(docker compose "${system_compose_args[@]}" config --format json)"

if [[ "${1:-}" == "--record-all" ]]; then
  printf '%s' "$compose_json" | STATE_FILE="$state_file" RECORD_ALL=true python3 -c '
import hashlib, json, os, subprocess, sys
config = json.load(sys.stdin)
state_path = os.environ["STATE_FILE"]
services = config.get("services", {})
def digest(paths):
    names = set()
    for path in paths:
        names.update(x for x in subprocess.check_output(["git", "ls-files", "-z", "--", path]).decode().split("\0") if x)
        names.update(x for x in subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z", "--", path]).decode().split("\0") if x)
    h = hashlib.sha256()
    for path in sorted(p for p in names if os.path.isfile(p)):
        blob = subprocess.check_output(["git", "hash-object", "--", path]).decode().strip()
        h.update(path.encode() + b"\0" + blob.encode() + b"\0")
    return h.hexdigest()
def crate_for(service):
    build = service.get("build") or {}
    crate = (build.get("args") or {}).get("CRATE")
    if crate:
        return crate
    context = build.get("context", "")
    if context.endswith("src/rust/insights"):
        return "insights"
    return None
common = digest(["system/compose.yml", "system/Dockerfile.rust"])
state = {name: common + ":" + digest(["src/rust/" + crate]) for name, svc in services.items() if (crate := crate_for(svc))}
with open(state_path, "w") as f:
    json.dump(state, f, indent=2, sort_keys=True)
    f.write("\n")
'
  echo "Recorded current build inputs as deployed for configured system services."
  exit 0
fi

if (($# > 0)); then
  echo "Usage: $0 [--record-all]" >&2
  exit 2
fi

mapfile -t services < <(printf '%s' "$compose_json" | STATE_FILE="$state_file" python3 -c '
import hashlib, json, os, subprocess, sys
config = json.load(sys.stdin)
state_path = os.environ["STATE_FILE"]
services = config.get("services", {})
try:
    with open(state_path) as f: deployed = json.load(f)
except FileNotFoundError:
    deployed = {}

def current_digest(paths):
    names = set()
    for path in paths:
        tracked = subprocess.check_output(["git", "ls-files", "-z", "--", path]).decode().split("\0")
        untracked = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z", "--", path]).decode().split("\0")
        names.update(p for p in tracked + untracked if p and os.path.isfile(p))
    h = hashlib.sha256()
    for path in sorted(names):
        blob = subprocess.check_output(["git", "hash-object", "--", path]).decode().strip()
        h.update(path.encode() + b"\0" + blob.encode() + b"\0")
    return h.hexdigest()

def head_digest(paths):
    names = set()
    for path in paths:
        result = subprocess.check_output(["git", "ls-tree", "-r", "-z", "HEAD", "--", path]).decode().split("\0")
        for entry in result:
            if entry:
                metadata, name = entry.split("\t", 1)
                names.add((name, metadata.split()[2]))
    h = hashlib.sha256()
    for name, blob in sorted(names):
        h.update(name.encode() + b"\0" + blob.encode() + b"\0")
    return h.hexdigest()

def crate_for(service):
    build = service.get("build") or {}
    crate = (build.get("args") or {}).get("CRATE")
    if crate:
        return crate
    if (build.get("context", "")).endswith("src/rust/insights"):
        return "insights"
    return None

common_paths = ["system/compose.yml", "system/Dockerfile.rust"]
common = current_digest(common_paths)
head_common = head_digest(common_paths)
stale = []
current = {}
for name, svc in services.items():
    crate = crate_for(svc)
    if not crate:
        continue
    fingerprint = common + ":" + current_digest(["src/rust/" + crate])
    baseline = deployed.get(name, head_common + ":" + head_digest(["src/rust/" + crate]))
    current[name] = fingerprint
    if fingerprint != baseline:
        stale.append(name)

for name in stale:
    print(name)
'
)

if ((${#services[@]} == 0)); then
  echo "No system service build inputs differ from the last recorded deployment."
  exit 0
fi

printf 'Rebuilding changed services: %s\n' "${services[*]}"
docker compose "${system_compose_args[@]}" up -d --build --force-recreate --no-deps "${services[@]}"
printf '%s' "$compose_json" | STATE_FILE="$state_file" DEPLOYED_SERVICES="${services[*]}" python3 -c '
import hashlib, json, os, subprocess, sys
config = json.load(sys.stdin)
path = os.environ["STATE_FILE"]
try:
    with open(path) as f: state = json.load(f)
except FileNotFoundError:
    state = {}
selected = set(os.environ["DEPLOYED_SERVICES"].split())
def digest(paths):
    names = set()
    for p in paths:
        names.update(x for x in subprocess.check_output(["git", "ls-files", "-z", "--", p]).decode().split("\0") if x)
        names.update(x for x in subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z", "--", p]).decode().split("\0") if x)
    h = hashlib.sha256()
    for name in sorted(n for n in names if os.path.isfile(n)):
        blob = subprocess.check_output(["git", "hash-object", "--", name]).decode().strip()
        h.update(name.encode() + b"\0" + blob.encode() + b"\0")
    return h.hexdigest()
common = digest(["system/compose.yml", "system/Dockerfile.rust"])
for name in selected:
    svc = config["services"].get(name, {})
    build = svc.get("build") or {}
    crate = (build.get("args") or {}).get("CRATE")
    if not crate and build.get("context", "").endswith("src/rust/insights"):
        crate = "insights"
    if crate:
        state[name] = common + ":" + digest(["src/rust/" + crate])
with open(path, "w") as f:
    json.dump(state, f, indent=2, sort_keys=True)
    f.write("\n")
'
echo "Changed services rebuilt, restarted, and recorded as deployed."
