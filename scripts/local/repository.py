import argparse
import re
import json
import tomllib
from collections import defaultdict
from common import ROOT, git_files, changed_files, read_text, emit, save

SOURCE_SUFFIXES = {".rs", ".ts", ".tsx", ".js", ".jsx"}

def relative(path):
    return str(path.relative_to(ROOT))

def source_files():
    return [
        p for p in git_files()
        if p.suffix in SOURCE_SUFFIXES
        and p.is_file()
        and p.stat().st_size <= 2000000
        and not any(
            x in p.parts for x in
            ("vendor", "node_modules", "target", "dist")
        )
    ]

def dependency_map():
    result = []
    for p in git_files():
        try:
            if p.name == "Cargo.toml":
                data = tomllib.loads(read_text(p).lstrip("\ufeff"))
                package = data.get("package", {}).get("name")
                if package:
                    dependencies = set()
                    for section in (
                        "dependencies", "dev-dependencies",
                        "build-dependencies"
                    ):
                        dependencies.update(data.get(section, {}))
                    result.append({
                        "name": package,
                        "kind": "rust",
                        "manifest": relative(p),
                        "dependencies": sorted(dependencies)
                    })

            elif p.name == "package.json":
                data = json.loads(read_text(p))
                dependencies = set()
                for section in (
                    "dependencies", "devDependencies",
                    "peerDependencies"
                ):
                    dependencies.update(data.get(section, {}))
                result.append({
                    "name": data.get("name"),
                    "kind": "typescript",
                    "manifest": relative(p),
                    "dependencies": sorted(dependencies)
                })
        except Exception as e:
            result.append({
                "manifest": relative(p),
                "error": str(e)
            })
    return result

def impact():
    changed = changed_files()
    packages = dependency_map()
    affected = set()
    for pkg in packages:
        directory = str(
            (ROOT/pkg["manifest"]).parent.relative_to(ROOT)
        )
        if pkg.get("name") and any(
            directory == "." or
            f.startswith(directory+"/") or f == pkg["manifest"]
            for f in changed
        ):
            affected.add(pkg["name"])

    direct = sorted(affected)
    while True:
        previous = len(affected)
        for pkg in packages:
            if pkg.get("name") and affected.intersection(
                pkg.get("dependencies", [])
            ):
                affected.add(pkg["name"])
        if len(affected) == previous:
            break

    return {
        "changed_files": changed,
        "direct_packages": direct,
        "potentially_affected_packages": sorted(affected),
        "rust_packages": sorted({
            p["name"] for p in packages
            if p.get("kind") == "rust"
            and p.get("name") in affected
        }),
        "confidence": "HEURISTIC",
        "mandatory_integration_tests_still_required": True
    }

def symbols(term=None):
    pattern = re.compile(
        r"\b(fn|struct|enum|trait|type|interface|class|function)"
        r"\s+([A-Za-z_]\w*)"
    )
    result = []
    for p in source_files():
        for n,line in enumerate(read_text(p).splitlines(), 1):
            for m in pattern.finditer(line):
                if not term or term.lower() in m[2].lower():
                    result.append({
                        "symbol": m[2], "kind": m[1],
                        "file": relative(p), "line": n
                    })
    return result

def matches(term):
    pattern = re.compile(r"\b"+re.escape(term)+r"\b")
    result = []
    for p in source_files():
        for n,line in enumerate(read_text(p).splitlines(), 1):
            if pattern.search(line):
                result.append({
                    "file": relative(p),
                    "line": n,
                    "text": line.strip()[:240]
                })
    return result

def api_contracts():
    routes = defaultdict(list)
    pattern = re.compile(
        r'["\x27](/(?:signals|positions|internal|summary|profiles)'
        r'[^"\x27\s]{0,160})["\x27]'
    )
    for p in source_files():
        for n,line in enumerate(read_text(p).splitlines(), 1):
            for route in pattern.findall(line):
                routes[route].append({
                    "file": relative(p), "line": n,
                    "language": p.suffix
                })
    return dict(routes)

def test_map():
    result = []
    for p in source_files():
        lines = read_text(p).splitlines()
        for i,line in enumerate(lines):
            if (
                re.search(r"#\[(?:tokio::)?test\b", line)
                or re.search(r"\b(?:it|test)\s*\(", line)
            ):
                result.append({
                    "file": relative(p),
                    "line": i+1,
                    "text": " ".join(lines[i:i+3])[:250]
                })
    return result

def config_inventory():
    result = []
    for p in git_files():
        if (
            p.suffix in (".toml", ".yaml", ".yml")
            or p.name in ("AGENTS.md", "justfile")
            or "settings" in p.stem.lower()
            or "config" in p.stem.lower()
        ):
            result.append(relative(p))
    # Paths only: do not dump secrets or configuration values.
    return result

def main(name):
    p = argparse.ArgumentParser(description=name)
    p.add_argument("--term")
    p.add_argument("--limit", type=int, default=50)
    a = p.parse_args()

    if not 1 <= a.limit <= 500:
        p.error("--limit must be 1-500")

    if name == "dependency_map":
        result = dependency_map()
    elif name in ("change_impact", "affected_tests"):
        result = impact()
    elif name == "symbol_index":
        result = symbols(a.term)
    elif name == "callers":
        if not a.term:
            p.error("--term required")
        result = matches(a.term)
    elif name == "api_contracts":
        result = api_contracts()
    elif name == "test_map":
        result = test_map()
    elif name == "config_inventory":
        result = config_inventory()
    elif name == "coverage_gaps":
        result = {
            "status": "REVIEW_REQUIRED",
            "changed": changed_files(),
            "test_locations": test_map()[:a.limit],
            "note": "Test locations are not measured code coverage."
        }
    elif name == "architecture_check":
        result = {
            "status": "BLOCKED",
            "reason": "Explicit architecture rules not configured.",
            "dependency_inventory": save(dependency_map())
        }
    else:
        raise ValueError(name)

    evidence = save(result, name)
    emit({
        "analysis": name,
        "confidence": "STATIC_HEURISTIC",
        "evidence": evidence,
        "count": len(result),
        "preview": result[:a.limit]
                   if isinstance(result, list) else result
    })
    return 1 if isinstance(result, dict) and result.get(
        "status"
    ) == "BLOCKED" else 0
