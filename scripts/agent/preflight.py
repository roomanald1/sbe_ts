#!/usr/bin/env python3

import ast
import json
import shutil
import subprocess
from pathlib import Path

try:
    import tomllib
except ImportError:
    tomllib = None

ROOT = Path.home() / "dev/sbe_ts"
HOME = Path.home()

counts = {"PASS": 0, "WARN": 0, "FAIL": 0}

def report(status, name, detail=""):
    counts[status] += 1
    print(f"[{status:4}] {name:38} {detail}")

def check(ok, name, detail=""):
    report("PASS" if ok else "FAIL", name, detail)

def run(*cmd):
    try:
        p = subprocess.run(
            cmd, cwd=ROOT, capture_output=True,
            text=True, timeout=30
        )
        return p.returncode, p.stdout.strip(), p.stderr.strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return 1, "", str(e)

def section(name):
    print(f"\n=== {name} ===")

section("REQUIRED TOOLS")

for tool in ("codex", "bd", "rtk", "git", "python3",
             "rg", "docker"):
    path = shutil.which(tool)
    check(bool(path), tool, path or "Not installed")

for tool in ("codex", "bd", "rtk"):
    rc, out, err = run(tool, "--version")
    report("PASS" if rc == 0 else "WARN",
           f"{tool} version", (out or err)[:150])

section("BEADS")

rc, out, err = run("bd", "prime")
size = len(out.encode())

check(rc == 0 and size > 0,
      "Beads priming works", f"{size:,} bytes")

if rc == 0:
    report("PASS" if size <= 5000 else "WARN",
           "Beads priming size",
           f"{size:,} bytes; target <= 5,000")

check((ROOT / ".beads/PRIME.md").is_file(),
      "Compact PRIME.md")

rc, config, err = run("bd", "config", "list")

for key in ("prime.max-memories",
            "prime.max-memory-chars"):
    matches = [
        line.strip() for line in config.splitlines()
        if key in line
    ]
    report("PASS" if matches else "WARN",
           key, "; ".join(matches) or "Not confirmed")

rc, out, err = run("bd", "memories")
check(rc == 0, "Persistent memory retrieval",
      f"{len(out.encode()):,} bytes returned")

section("CODEX HOOKS")

hooks_path = ROOT / ".codex/hooks.json"

try:
    hooks = json.loads(hooks_path.read_text())["hooks"]

    expected = {
        "SessionStart": "bd codex-hook SessionStart",
        "PreCompact": "bd codex-hook PreCompact",
        "PostCompact": "bd codex-hook PostCompact",
        "UserPromptSubmit": "bd codex-hook UserPromptSubmit",
        "PreToolUse": "rtk hook codex",
    }

    for event, command in expected.items():
        commands = [
            hook.get("command", "")
            for group in hooks.get(event, [])
            for hook in group.get("hooks", [])
        ]
        check(command in commands, event, command)

except Exception as e:
    report("FAIL", "Hook configuration", str(e))

section("AGENT INSTRUCTIONS")

path = ROOT / "AGENTS.md"
instructions = path.read_text() if path.exists() else ""

check(bool(instructions), "Repository AGENTS.md",
      f"{len(instructions.encode()):,} bytes")

for keyword in ("rtk", "bd remember", "bd memories",
                "scripts/diagnostics", "scripts/agent"):
    report(
        "PASS" if keyword.lower() in instructions.lower()
        else "WARN",
        f"AGENTS.md references {keyword}"
    )

section("AGENT PROFILES")

profiles = {
    "coordinator": ("high", 131072, 80000),
    "architect": ("high", 98304, 60000),
    "quantitative": ("high", 98304, 60000),
    "financial": ("high", 98304, 60000),
    "rust_worker": ("medium", 65536, 40000),
    "integration": ("medium", 65536, 40000),
    "react_worker": ("medium", 49152, 30000),
    "explorer": ("low", 32768, 20000),
    "documentation": ("low", 32768, 20000),
}

active = ROOT / ".codex/agents"
staging = HOME / ".codex-agent-staging/agents"

for name, expected in profiles.items():
    path = active / f"{name}.toml"
    location = "active"

    if not path.exists():
        path = staging / f"{name}.toml"
        location = "staged"

    if not path.exists():
        report("FAIL", name, "Profile missing")
        continue

    if tomllib is None:
        report("WARN", name, "Python 3.11+ needed for TOML validation")
        continue

    try:
        data = tomllib.loads(path.read_text())

        actual = (
            data.get("model_reasoning_effort"),
            data.get("model_context_window"),
            data.get("model_auto_compact_token_limit"),
        )

        valid = (
            actual == expected
            and data.get("model") == "gpt-6-luna"
            and data.get("name") == name
            and bool(data.get("developer_instructions"))
        )

        report(
            "PASS" if valid and location == "active" else "WARN",
            name,
            f"{location}: {actual}; "
            f"model={data.get('model')}; "
            f"matches_expected={valid}"
        )

    except Exception as e:
        report("FAIL", name, str(e))

check((active / "profile_probe.toml").exists(),
      "Runtime verification probe")

section("CODEX CONFIGURATION")

for path in (
    HOME / ".codex/config.toml",
    ROOT / ".codex/config.toml",
    HOME / ".codex-agent-staging/config.fragment.toml",
):
    if not path.exists():
        continue

    print(f"\n{path}")

    if tomllib is None:
        continue

    try:
        data = tomllib.loads(path.read_text())

        for key in (
            "model",
            "model_reasoning_effort",
            "model_context_window",
            "model_auto_compact_token_limit",
        ):
            print(f"  {key}: {data.get(key, 'not set')}")

        agents = data.get("agents", {})
        print(f"  agents.max_threads: {agents.get('max_threads', 'not set')}")
        print(f"  agents.max_depth: {agents.get('max_depth', 'not set')}")

    except Exception as e:
        report("FAIL", "TOML configuration", str(e))

section("PYTHON DIAGNOSTICS")

scripts = {
    "agent": [
        "rollouts", "agent_usage", "context_audit",
        "token_trends", "repo_index",
    ],
    "diagnostics": [
        "common", "test_summary", "system_health",
        "sqlite_inspect", "trade_reconciliation",
        "strategy_performance", "learning_coverage",
        "scout_health", "capital_allocation",
        "walk_forward_validation", "missed_opportunities",
        "paper_soak", "latency_summary",
    ],
}

for folder, names in scripts.items():
    for name in names:
        path = ROOT / "scripts" / folder / f"{name}.py"

        try:
            ast.parse(path.read_text())
            report("PASS", f"{name}.py", "Syntax valid")
        except Exception as e:
            report("FAIL", f"{name}.py", str(e))

for name in ("README.md", "CODEX_INSTRUCTIONS.md"):
    check((ROOT / "scripts/diagnostics" / name).exists(),
          f"Diagnostics {name}")

section("MCP")

rc, out, err = run("codex", "mcp", "list")
check(rc == 0, "MCP configuration readable")
print(out[:2000])

section("RTK SAVINGS")

rc, out, err = run("rtk", "gain")
print((out or err)[:2000])

section("DOCKER")

rc, out, err = run(
    "docker", "ps", "-a",
    "--format", "{{json .}}"
)

if rc == 0:
    rows = [json.loads(line) for line in out.splitlines()]
    running = [
        r for r in rows
        if r.get("State") == "running"
    ]
    unhealthy = [
        r for r in running
        if "(unhealthy)" in r.get("Status", "")
    ]

    report("PASS", "Docker accessible",
           f"{len(running)}/{len(rows)} containers running")

    report("WARN" if unhealthy else "PASS",
           "Running container health",
           f"{len(unhealthy)} unhealthy")

    for r in unhealthy:
        print("  ", r.get("Names"))

    for r in rows:
        if r.get("State") != "running":
            print("  Not running:", r.get("Names"))

else:
    report("FAIL", "Docker accessible", err[:200])

section("RESOURCES")

rc, out, err = run("free", "-h")
print(out)

rc, out, err = run("df", "-h", ".")
print(out)

section("REMAINING RUNTIME VERIFICATION")

for name in (
    "Native VS Code named agent delegation",
    "Effective child reasoning settings",
    "Effective child context limits",
    "Effective compaction thresholds",
    "Agent concurrency and depth limits",
    "Token accounting against original monitor",
    "Actual Beads hook injection size",
    "Trading diagnostic data adapters",
):
    report("WARN", name, "Not established by static checks")

section("GIT STATUS")

rc, out, err = run("git", "status", "--short")
print(out[:3000] or "Clean")

section("PREFLIGHT SUMMARY")

for status, count in counts.items():
    print(f"{status}: {count}")

if counts["FAIL"]:
    print("\nRESULT: FIX FAILED STATIC CHECKS")
else:
    print("\nRESULT: STATIC PREFLIGHT PASSED")
    print("Review warnings before autonomous execution.")

print("No configuration or trading data modified.")
print("Runtime verification remains mandatory.")
