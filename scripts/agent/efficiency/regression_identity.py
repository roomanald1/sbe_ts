import argparse
import hashlib
import json
import os
import platform
import subprocess
from common import ROOT, git, save

p = argparse.ArgumentParser()
p.add_argument("--compare")
p.add_argument("--command", default="")
a = p.parse_args()

paths = subprocess.check_output(
    ["git", "-C", str(ROOT), "ls-files",
     "-z", "--cached", "--others",
     "--exclude-standard"]
).decode(errors="surrogateescape").split("\0")

digest = hashlib.sha256()
skipped = []
count = 0

for relative in sorted(set(filter(None, paths))):
    path = ROOT / relative

    if path.is_symlink() or not path.is_file():
        skipped.append(relative)
        continue

    if path.stat().st_size > 32000000:
        skipped.append(relative)
        continue

    before = path.stat()
    content = path.read_bytes()
    after = path.stat()

    if (
        before.st_mtime_ns != after.st_mtime_ns
        or before.st_size != after.st_size
    ):
        skipped.append(relative)
        continue

    digest.update(relative.encode(errors="surrogateescape"))
    digest.update(b"\0")
    digest.update(hashlib.sha256(content).digest())
    count += 1

versions = {}

for command in [
    ["rustc", "--version"],
    ["cargo", "--version"],
    ["node", "--version"],
    ["just", "--version"]
]:
    try:
        versions[command[0]] = subprocess.check_output(
            command, text=True, timeout=10
        ).strip()
    except (OSError, subprocess.SubprocessError):
        versions[command[0]] = None

identity = {
    "head": git("rev-parse", "HEAD"),
    "content_sha256": digest.hexdigest(),
    "file_count": count,
    "skipped": skipped,
    "platform": platform.platform(),
    "versions": versions,
    "command": a.command,
    "environment": {
        key: os.environ.get(key)
        for key in [
            "RUSTFLAGS", "CARGO_TARGET_DIR",
            "RUSTUP_TOOLCHAIN", "CARGO_BUILD_TARGET"
        ]
    }
}

result = {
    "identity": identity,
    "status": (
        "PARTIAL_FINGERPRINT"
        if skipped else "FINGERPRINT_COLLECTED"
    ),
    "reuse_authorized": False,
    "limitations": [
        "Conservative repository-wide fingerprint.",
        "Not an atomic snapshot.",
        "External fixtures and runtime state not captured.",
        "Does not establish successful prior test execution."
    ]
}

if a.compare:
    with open(a.compare) as f:
        previous = json.load(f)

    result["identity_matches"] = (
        previous.get("identity") == identity
        and not skipped
    )

    result["status"] = (
        "REVIEW_REUSE_CANDIDATE"
        if result["identity_matches"]
        else "INPUTS_CHANGED_OR_INCOMPLETE"
    )

print(json.dumps(result, indent=2))
print("REPORT:", save("regression_identity", result))
