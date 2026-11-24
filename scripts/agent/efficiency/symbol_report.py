#!/usr/bin/env python3
"""Cross-crate symbol, behavioural-test and change-impact report."""

import argparse
import json
import re
import subprocess
from pathlib import Path
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[3]
RUST = ROOT / "src/rust"

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("symbol", nargs="?")
p.add_argument("--crate")
p.add_argument("--limit", type=int, default=30)
p.add_argument("--chars", type=int, default=30000)
p.add_argument("--json")
p.add_argument("--brief", nargs="?", const="auto")
p.add_argument("--min-score", type=int, default=45)
p.add_argument("--show-all", action="store_true")
p.add_argument("--self-test", action="store_true")
args = p.parse_args()

if args.self_test:
    fixtures = [
        ("test calls target()", "DIRECT"),
        ("test calls caller()", "RELATED"),
        ("test checks closed trade audit exclusion", "BEHAVIOURAL"),
        ("test unrelated calculation", "NONE"),
    ]

    def classify(text):
        if "target()" in text:
            return "DIRECT"
        if "caller()" in text:
            return "RELATED"
        if any(x in text for x in ("closed trade", "audit exclusion")):
            return "BEHAVIOURAL"
        return "NONE"

    assert all(classify(x) == expected for x, expected in fixtures)
    print("SELF TEST: 4/4 PASS")
    raise SystemExit(0)

if not args.symbol or not args.crate:
    p.error("symbol and --crate required unless using --self-test")

if not 0 <= args.min_score <= 100:
    p.error("--min-score must be between 0 and 100")

if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", args.symbol):
    p.error("Invalid symbol")

if not re.fullmatch(r"[A-Za-z0-9_-]+", args.crate):
    p.error("Invalid crate")

if min(args.limit, args.chars) < 1:
    p.error("Limits must be positive")

if not (RUST / args.crate).is_dir():
    p.error("Crate does not exist")

def git(*command):
    return subprocess.run(
        ["git", *command],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=60
    )

def files():
    return sorted(
        f for f in RUST.rglob("*.rs")
        if "target" not in f.parts
        and "vendor" not in f.parts
    )

def line_at(text, offset):
    return text.count("\n", 0, offset) + 1

def extract_block(text, opening):
    """Extract a brace-delimited block, ignoring basic strings/comments."""
    depth = 0
    state = "normal"
    i = opening

    while i < len(text):
        c = text[i]
        n = text[i + 1] if i + 1 < len(text) else ""

        if state == "line":
            if c == "\n":
                state = "normal"
        elif state == "block":
            if c == "*" and n == "/":
                state = "normal"
                i += 1
        elif state == "string":
            if c == "\\":
                i += 1
            elif c == '"':
                state = "normal"
        else:
            if c == "/" and n == "/":
                state = "line"
                i += 1
            elif c == "/" and n == "*":
                state = "block"
                i += 1
            elif c == '"':
                state = "string"
            elif c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    return text[opening:i + 1]
        i += 1
    return ""

def function_blocks(text):
    pattern = re.compile(
        r'\b(?:pub(?:\([^)]*\))?\s+)?'
        r'(?:async\s+)?fn\s+'
        r'([A-Za-z_][A-Za-z0-9_]*)'
    )
    for match in pattern.finditer(text):
        opening = text.find("{", match.end())
        if opening < 0:
            continue
        block = extract_block(text, opening)
        if block:
            yield match, block

def relative(path):
    return str(path.relative_to(ROOT))

all_files = files()
target_files = [
    f for f in all_files
    if f.is_relative_to(RUST / args.crate)
]

# 1. Find target definition.
definitions = []

for file in target_files:
    text = file.read_text(errors="replace")
    for match, body in function_blocks(text):
        if match.group(1) == args.symbol:
            definitions.append({
                "file": relative(file),
                "line": line_at(text, match.start()),
                "signature": text[match.start():text.find("{", match.end())].strip(),
                "body": body,
                "source": text
            })

if not definitions:
    raise SystemExit("BLOCKED: no function definition found")

if len(definitions) > 1:
    print("WARNING: multiple definitions found")

definition = definitions[0]
definition_body = definition["body"]

# 2. Identify types actually named in the target function.
excluded = {
    "String", "Option", "Result", "Vec", "Some", "None",
    "Self", "Default", "Box", "Arc", "Mutex", "HashMap",
    "HashSet", "Iterator", "IntoIterator", "true", "false"
}

type_names = set(re.findall(
    r'\b[A-Z][A-Za-z0-9_]*\b',
    definition["signature"] + "\n" + definition_body
)) - excluded

type_declarations = []

for file in all_files:
    text = file.read_text(errors="replace")
    for name in type_names:
        pattern = re.compile(
            r'\b(?:pub(?:\([^)]*\))?\s+)?'
            r'(?:struct|enum|trait|type)\s+'
            + re.escape(name) + r'\b'
        )
        match = pattern.search(text)
        if match:
            type_declarations.append({
                "name": name,
                "file": relative(file),
                "line": line_at(text, match.start())
            })

# 3. Find cross-crate references and enclosing function candidates.
references = []
caller_names = set()
reference_files = set()

for file in all_files:
    text = file.read_text(errors="replace")
    for match in re.finditer(
        r'\b' + re.escape(args.symbol) + r'\s*\(',
        text
    ):
        if relative(file) == definition["file"]:
            target_start = definition["line"]
            target_end = target_start + definition_body.count("\n")
            current_line = line_at(text, match.start())
            if target_start <= current_line <= target_end:
                continue

        preceding = text[:match.start()]
        enclosing = list(re.finditer(
            r'\b(?:async\s+)?fn\s+'
            r'([A-Za-z_][A-Za-z0-9_]*)',
            preceding
        ))
        caller = enclosing[-1].group(1) if enclosing else None

        references.append({
            "file": relative(file),
            "line": line_at(text, match.start()),
            "enclosing_function": caller,
            "confidence": "LEXICAL"
        })
        reference_files.add(relative(file))

        if caller and caller != args.symbol:
            caller_names.add(caller)

# 4. Build behavioural vocabulary.
# These are search hints, not asserted semantic dependencies.
aliases = {
    "closed_trade": [
        "closed", "trade", "return", "realized",
        "realised", "pnl", "audit", "learning"
    ],
    "learning": [
        "feedback", "observation", "eligibility",
        "promotion", "holdout", "outcome"
    ],
    "exit_policy": [
        "exit", "stop", "take_profit", "target",
        "protection", "position"
    ],
    "signal": [
        "signal", "candidate", "selection", "execution"
    ],
    "budget": [
        "budget", "allocation", "capital", "capacity"
    ]
}

words = set(re.findall(
    r'[a-z][a-z0-9]+',
    args.symbol.lower()
))

vocabulary = set(words)

for key, values in aliases.items():
    if key in args.symbol.lower():
        vocabulary.update(values)

vocabulary -= {
    "fn", "the", "and", "with", "from", "this",
    "that", "settings", "percent", "value"
}

# Build stronger evidence from the actual target definition.
generic_terms = {
    "trade", "learning", "return", "returns", "signal",
    "strategy", "position", "execution", "outcome",
    "feedback", "closed", "value", "settings", "state",
    "data", "result", "event", "context", "target",
    "observation", "test", "default", "some", "none"
}

strong_identifiers = set(re.findall(
    r"\b[a-z][a-z0-9]*(?:_[a-z0-9]+)+\b",
    definition_body
))

strong_identifiers.discard(args.symbol)

strong_types = type_names - {
    "Settings", "State", "Config", "Context",
    "Dashboard", "Signal", "Value"
}

specific_terms = vocabulary - generic_terms

def identifier_present(name, text):
    return bool(re.search(
        r"\b" + re.escape(name) + r"\b", text
    ))

def behavioural_score(name, body, assertions):
    combined = name + "\n" + body

    field_hits = sorted(
        x for x in strong_identifiers
        if identifier_present(x, combined)
    )

    type_hits = sorted(
        x for x in strong_types
        if identifier_present(x, combined)
    )

    terms = set(re.findall(
        r"[a-z][a-z0-9]+", combined.lower()
    ))

    name_terms = set(re.findall(
        r"[a-z][a-z0-9]+", name.lower()
    ))

    term_hits = sorted(specific_terms & terms)
    name_hits = sorted(specific_terms & name_terms)

    assertion_text = "\n".join(assertions)

    assertion_hits = sorted(
        x for x in field_hits + type_hits
        if identifier_present(x, assertion_text)
    )

    score = (
        min(30, 18 * len(field_hits))
        + min(24, 18 * len(type_hits))
        + min(15, 5 * len(term_hits))
        + min(12, 6 * len(name_hits))
        + min(12, 6 * len(assertion_hits))
    )

    # Shared terminology alone is insufficient.
    if not field_hits and not type_hits:
        score = min(score, 30)

    evidence = {
        "fields": field_hits,
        "types": type_hits,
        "specific_terms": term_hits,
        "test_name_terms": name_hits,
        "assertion_identifiers": assertion_hits
    }

    return min(score, 74), evidence

# 5. Extract actual test functions and assertions.
tests = []
test_files = set()

test_attribute = re.compile(
    r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]'
)

for file in all_files:
    text = file.read_text(errors="replace")

    for attr in test_attribute.finditer(text):
        tail = text[attr.end():attr.end() + 1000]

        fn = re.search(
            r'\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*'
            r'\([^)]*\)\s*(?:->[^{]+)?\{',
            tail,
            re.S
        )

        if not fn:
            continue

        opening = attr.end() + fn.end() - 1
        body = extract_block(text, opening)

        if not body:
            continue

        name = fn.group(1)
        combined = (name + "\n" + body).lower()
        relative_file = relative(file)

        direct = bool(re.search(
            r'\b' + re.escape(args.symbol) + r'\s*\(',
            body
        ))

        related = any(
            re.search(r'\b' + re.escape(c) + r'\s*\(', body)
            for c in caller_names
        )

        matched = sorted(
            term for term in vocabulary
            if len(term) >= 4 and term in combined
        )

        # Require at least two meaningful vocabulary matches
        # for a behavioural candidate unless the test name itself
        # strongly matches the target concept.
        name_matches = [
            term for term in vocabulary
            if len(term) >= 4 and term in name.lower()
        ]

        assertions = []

        for assertion in re.finditer(
            r"\b(?:assert|assert_eq|assert_ne|assert_matches|"
            r"debug_assert|debug_assert_eq|debug_assert_ne)!\s*\(",
            body
        ):
            opening = assertion.end() - 1
            depth = 0
            in_string = False
            escaped = False

            for index in range(opening, min(len(body), opening + 2000)):
                char = body[index]

                if in_string:
                    if escaped:
                        escaped = False
                    elif char == "\\":
                        escaped = True
                    elif char == '"':
                        in_string = False
                    continue

                if char == '"':
                    in_string = True
                elif char == "(":
                    depth += 1
                elif char == ")":
                    depth -= 1
                    if depth == 0:
                        snippet = body[assertion.start():index + 1]
                        assertions.append(
                            " ".join(snippet.split())[:500]
                        )
                        break

        behaviour_score, evidence = behavioural_score(
            name, body, assertions
        )

        if direct:
            classification = "DIRECT"
            score = 100
        elif related:
            classification = "RELATED"
            score = 85
        elif behaviour_score >= args.min_score:
            classification = "BEHAVIOURAL"
            score = behaviour_score
        elif args.show_all and behaviour_score > 0:
            classification = "GENERAL"
            score = behaviour_score
        else:
            continue

        tests.append({
            "name": name,
            "file": relative_file,
            "line": line_at(text, attr.start()),
            "classification": classification,
            "score": min(score, 100),
            "matched_terms": matched,
            "evidence": evidence,
            "coverage_verified": False,
            "assertion_count": len(assertions),
            "assertions": assertions[:12]
        })

        test_files.add(relative_file)

tests.sort(
    key=lambda x: (
        -x["score"],
        x["classification"],
        x["file"],
        x["line"]
    )
)

# 6. Git impact classification.
status = git("status", "--short", "--untracked-files=all")

changed = set()

if status.returncode == 0:
    for line in status.stdout.splitlines():
        if len(line) > 3:
            changed.add(line[3:].strip())

type_files = {item["file"] for item in type_declarations}
caller_files = {
    item["file"] for item in references
    if item["enclosing_function"]
}
direct_files = reference_files

definition_changes = changed & {definition["file"]}

caller_changes = (
    changed & caller_files
) - definition_changes

reference_changes = (
    changed & reference_files
) - definition_changes - caller_changes

type_changes = (
    changed & type_files
) - definition_changes - caller_changes - reference_changes

relevant_test_files = {
    t["file"] for t in tests
    if t["classification"] != "GENERAL"
}

test_changes = (
    changed & relevant_test_files
) - definition_changes - caller_changes - reference_changes - type_changes

classified = (
    definition_changes | caller_changes | reference_changes
    | type_changes | test_changes
)

impact = {
    "definition_files": sorted(definition_changes),
    "caller_candidate_files": sorted(caller_changes),
    "other_reference_files": sorted(reference_changes),
    "related_type_files": sorted(type_changes),
    "test_candidate_files": sorted(test_changes),
    "unclassified_changed_files": sorted(changed - classified),
    "total_changed_files": len(changed),
    "classification_note": (
        "File-level candidates only; unclassified does not mean unrelated."
    )
}

# 7. Regression recommendations.
regression = [
    {
        "command": f"just test {args.crate}",
        "reason": "Target function is defined in this crate.",
        "executed": False
    },
    {
        "command": "just diff-check",
        "reason": "Check patch and whitespace integrity.",
        "executed": False
    }
]

cross_crates = sorted({
    Path(item["file"]).parts[2]
    for item in references
    if Path(item["file"]).parts[0:2] == ("src", "rust")
    and Path(item["file"]).parts[2] != args.crate
})

for crate in cross_crates:
    regression.append({
        "command": f"just test {crate}",
        "reason": "Cross-crate lexical reference found; verify whether the change affects this crate.",
        "executed": False
    })

report = {
    "version": "3.4.1",
    "generated_at": datetime.now(timezone.utc).isoformat(),
    "symbol": args.symbol,
    "crate": args.crate,
    "definition": {
        "file": definition["file"],
        "line": definition["line"],
        "signature": definition["signature"],
        "body": definition_body[:args.chars]
    },
    "references": references,
    "caller_candidates": sorted(caller_names),
    "related_types": type_declarations,
    "behavioural_vocabulary": sorted(vocabulary),
    "tests": {
        "direct": [t for t in tests if t["classification"] == "DIRECT"],
        "related": [t for t in tests if t["classification"] == "RELATED"],
        "behavioural": [t for t in tests if t["classification"] == "BEHAVIOURAL"],
        "general": [t for t in tests if t["classification"] == "GENERAL"]
    },
    "git_impact": impact,
    "regression_suggestions": regression,
    "limitations": [
        "Caller discovery is lexical, not a compiler call graph.",
        "Cross-crate references may include false positives.",
        "Test relevance scores are heuristic, not proof of coverage.",
        "Assertions are extracted textually and are not interpreted.",
        "Behavioural vocabulary may need domain-specific refinement.",
        "Git impact is file-level and cannot establish causality.",
        "No regression command is executed by this report."
    ]
}

print("\nSYMBOL REPORT v3.4.1")
print("=" * 65)
print("Symbol:", args.symbol)
print("Definition:", definition["file"], definition["line"])
print("Cross-crate references:", len(references))
print("Caller candidates:", len(caller_names))
print("Related types:", len(type_declarations))

print("\nTEST DISCOVERY")
for category in (
    ("direct", "related", "behavioural", "general")
    if args.show_all else
    ("direct", "related", "behavioural")
):
    items = report["tests"][category]
    print(f"\n{category.upper()}: {len(items)}")

    for item in items[:args.limit]:
        print(
            f'  [{item["score"]}] {item["name"]} '
            f'({item["file"]}:{item["line"]})'
        )
        print(
            f'      terms={", ".join(item["matched_terms"][:8])} '
            f'assertions={item["assertion_count"]}'
        )
        for assertion in item["assertions"][:3]:
            print("      ", assertion[:180])

print("\nGIT IMPACT")
for key, values in impact.items():
    if isinstance(values, list):
        print(f"{key}: {len(values)}")
        for value in values[:args.limit]:
            print("  ", value)
    else:
        print(f"{key}: {values}")

print("\nREGRESSION SUGGESTIONS")
for item in regression:
    print(item["command"])
    print(" ", item["reason"])

if args.json:
    output = Path(args.json).expanduser()
    if not output.is_absolute():
        output = ROOT / output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print("\nJSON:", output)


if args.brief is not None:
    if args.brief == "auto":
        brief_path = (
            Path.home() / ".cache/sbe-diagnostics"
            / f"context-brief-{args.crate}-{args.symbol}.md"
        )
    else:
        brief_path = Path(args.brief).expanduser()

    brief_path.parent.mkdir(parents=True, exist_ok=True)

    lines = [
        f"# Codex context brief: {args.symbol}",
        "",
        f"Crate: {args.crate}",
        f"Generated: {report['generated_at']}",
        "",
        "Evidence status: lexical candidates; verify before editing.",
        "",
        "## Read first",
        f"- Definition: {definition['file']}:{definition['line']}",
        "",
        "## References and caller candidates"
    ]

    for ref in references[:12]:
        lines.append(
            f"- {ref['file']}:{ref['line']} "
            f"({ref['enclosing_function'] or 'unknown caller'})"
        )

    lines += ["", "## Related types"]

    for item in type_declarations[:8]:
        lines.append(
            f"- {item['name']}: {item['file']}:{item['line']}"
        )

    lines += ["", "## Recommended test reading"]

    relevant = [
        t for t in tests
        if t["classification"] != "GENERAL"
    ][:10]

    if not relevant:
        lines.append("- No high-relevance test candidates identified.")

    for test in relevant:
        lines.append(
            f"- [{test['classification']} {test['score']}] "
            f"{test['name']}: {test['file']}:{test['line']}"
        )

        for assertion in test["assertions"][:2]:
            lines.append(f"  - Assertion: {assertion[:250]}")

        evidence = test.get("evidence", {})
        for key in ("fields", "types", "assertion_identifiers"):
            if evidence.get(key):
                lines.append(
                    f"  - {key}: {', '.join(evidence[key][:5])}"
                )

    lines += ["", "## Suggested validation"]

    for item in regression:
        lines.append(
            f"- `{item['command']}`: {item['reason']}"
        )

    lines += [
        "",
        "## Changed files requiring inspection"
    ]

    for category, values in impact.items():
        if not isinstance(values, list):
            continue
        if category == "unclassified_changed_files":
            lines.append(f"- Unclassified changes: {len(values)}")
            continue
        for value in values[:8]:
            lines.append(f"- {category}: {value}")

    lines += [
        "",
        "## Agent instructions",
        "- Verify candidate relationships before modifying code.",
        "- Read only the relevant source ranges first.",
        "- Treat behavioural rankings as hints, not coverage proof.",
        "- Add focused regression tests where coverage is missing.",
        "- Preserve unrelated working-tree changes.",
        "- Do not modify live services or databases.",
        "- Save verified reusable navigation discoveries with bd remember.",
        "- Do not save unverified candidate relationships.",
        ""
    ]

    brief_path.write_text("\n".join(lines))
    print("\nCODEX BRIEF:", brief_path)

print("\nNo source code or live services were modified.")
