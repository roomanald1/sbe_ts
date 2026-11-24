import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

parser = argparse.ArgumentParser(
    description="Bounded source symbol and caller navigation"
)
parser.add_argument("name")
parser.add_argument("--crate")
parser.add_argument("--lang", choices=["rust", "typescript", "tsx"],
                    default="rust")
parser.add_argument("--context", type=int, default=5)
parser.add_argument("--limit", type=int, default=30)
parser.add_argument("--chars", type=int, default=30000)
parser.add_argument("--references", action="store_true")
parser.add_argument("--definition", action="store_true")
parser.add_argument("--callers", action="store_true")
args = parser.parse_args()

if not re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$]*", args.name):
    parser.error("Invalid symbol name")

if min(args.context, args.limit, args.chars) < 1:
    parser.error("Limits must be positive")

base = ROOT / (
    "src/rust" if args.lang == "rust"
    else "src/typescript"
)

if args.crate:
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.crate):
        parser.error("Invalid crate")
    base /= args.crate

if not base.is_dir():
    parser.error(f"Directory not found: {base}")

rg = shutil.which("rg")
if not rg:
    parser.error("ripgrep is required")

suffixes = (
    ["rs"] if args.lang == "rust"
    else ["ts", "tsx", "js", "jsx"]
)

def run_rg(pattern, fixed=False):
    cmd = [rg, "-n", "--no-heading", "--color", "never"]

    for suffix in suffixes:
        cmd += ["--glob", f"*.{suffix}"]

    cmd += [
        "--glob", "!**/target/**",
        "--glob", "!**/node_modules/**",
    ]

    if fixed:
        cmd.append("-F")

    cmd += [pattern, str(base)]

    result = subprocess.run(
        cmd, capture_output=True, text=True
    )

    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr)

    return result.stdout.splitlines()

def declaration_pattern():
    if args.lang == "rust":
        return re.compile(
            r'^\s*(?:(?:pub(?:\([^)]*\))?|async|unsafe|'
            r'extern\s+"[^"]+")\s+)*'
            r'(?:fn|struct|enum|trait|type)\s+'
            + re.escape(args.name) + r'\b'
        )

    return re.compile(
        r'^\s*(?:(?:export|default|declare|async|abstract)\s+)*'
        r'(?:function|class|interface|type|const|let|var)\s+'
        + re.escape(args.name) + r'\b'
    )

# Find a balanced source block while ignoring ordinary strings
# and comments. Raw Rust strings are also recognised.
def matching_brace(text, opening):
    depth = 0
    i = opening
    state = "normal"
    block_depth = 0
    raw_end = None

    while i < len(text):
        c = text[i]
        n = text[i+1] if i+1 < len(text) else ""

        if state == "line_comment":
            if c == "\n":
                state = "normal"

        elif state == "block_comment":
            if c == "/" and n == "*":
                block_depth += 1
                i += 1
            elif c == "*" and n == "/":
                block_depth -= 1
                i += 1
                if block_depth == 0:
                    state = "normal"

        elif state == "string":
            if c == "\\":
                i += 1
            elif c == '"':
                state = "normal"

        elif state == "char":
            if c == "\\":
                i += 1
            elif c == "'":
                state = "normal"

        elif state == "raw":
            if text.startswith(raw_end, i):
                i += len(raw_end) - 1
                state = "normal"

        else:
            if c == "/" and n == "/":
                state = "line_comment"
                i += 1
            elif c == "/" and n == "*":
                state = "block_comment"
                block_depth = 1
                i += 1
            elif c == "r":
                m = re.match(r'r(#+)?"', text[i:])
                if m:
                    hashes = m.group(1) or ""
                    raw_end = '"' + hashes
                    i += len(m.group(0)) - 1
                    state = "raw"
            elif c == '"':
                state = "string"
            elif c == "'":
                # Lifetimes are not character literals.
                if not re.match(r"'[A-Za-z_][A-Za-z0-9_]*", text[i:]):
                    state = "char"
            elif c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    return i

        i += 1

    return None

def source_blocks(text):
    pattern = (
        r'(?m)^[ \t]*(?:(?:pub(?:\([^)]*\))?|async|unsafe|'
        r'const|default|extern\s+"[^"]+")\s+)*'
        r'(?:fn|struct|enum|trait|impl|mod|'
        r'function|class|interface)\s+'
        r'[A-Za-z_$][A-Za-z0-9_$]*'
    )

    for match in re.finditer(pattern, text):
        start = match.start()
        opening = text.find("{", match.end())

        if opening < 0:
            continue

        semicolon = text.find(";", match.end(), opening)

        if semicolon >= 0:
            continue

        closing = matching_brace(text, opening)

        if closing is None:
            continue

        header = text[start:opening]
        name_match = re.search(
            r'\b(fn|struct|enum|trait|impl|mod|'
            r'function|class|interface)\s+'
            r'([A-Za-z_$][A-Za-z0-9_$]*)',
            header
        )

        if not name_match:
            continue

        yield {
            "name": name_match.group(2),
            "kind": name_match.group(1),
            "start": start,
            "end": closing + 1
        }

def line_number(text, offset):
    return text.count("\n", 0, offset) + 1

# Exact lexical matches.
raw_matches = run_rg(args.name, fixed=True)
matches = []

for line in raw_matches:
    parts = line.split(":", 2)
    if len(parts) != 3:
        continue

    file, number, source = parts

    try:
        number = int(number)
    except ValueError:
        continue

    matches.append({
        "file": Path(file),
        "line": number,
        "source": source
    })

# Group matches by source file.
by_file = {}

for match in matches:
    by_file.setdefault(match["file"], []).append(match)

definitions = []
callers = []

for file, file_matches in by_file.items():
    text = file.read_text(errors="replace")
    relative = str(file.relative_to(ROOT))
    blocks = list(source_blocks(text))

    for block in blocks:
        if block["name"] == args.name:
            definitions.append({
                "file": relative,
                "line": line_number(text, block["start"]),
                "kind": block["kind"],
                "source": text[block["start"]:block["end"]]
            })

    for match in file_matches:
        offset = sum(
            len(line) + 1
            for line in text.splitlines()[:match["line"]-1]
        )

        enclosing = [
            b for b in blocks
            if b["start"] <= offset <= b["end"]
            and b["name"] != args.name
        ]

        if enclosing:
            block = min(
                enclosing,
                key=lambda b: b["end"] - b["start"]
            )

            callers.append({
                "file": relative,
                "line": line_number(text, block["start"]),
                "function": block["name"],
                "reference_line": match["line"]
            })

print(f"\nSYMBOL: {args.name}")
print(f"LEXICAL MATCHES: {len(matches)}")
print(f"DEFINITIONS: {len(definitions)}")
print(f"CALLER CANDIDATES: {len(callers)}")

remaining = args.chars

def emit(title, item, source):
    global remaining

    header = (
        f"\n{'='*65}\n"
        f"{title}: {item['file']}:{item['line']}\n"
    )

    if len(header) >= remaining:
        print("\nOUTPUT BUDGET EXHAUSTED")
        return

    print(header, end="")
    remaining -= len(header)

    if len(source) > remaining:
        source = source[:remaining] + "\n[OUTPUT TRUNCATED]"

    print(source)
    remaining -= len(source)

if args.definition:
    for item in definitions[:args.limit]:
        emit(
            f"DEFINITION ({item['kind']})",
            item,
            item["source"]
        )

if args.callers:
    seen = set()

    for item in callers[:args.limit]:
        key = (item["file"], item["function"])

        if key in seen:
            continue

        seen.add(key)

        print(
            f"\nCALLER CANDIDATE: {item['function']} "
            f"at {item['file']}:{item['line']}"
        )
        print(
            f"Reference at line {item['reference_line']}"
        )

        if remaining <= 0:
            break

        path = ROOT / item["file"]
        lines = path.read_text(errors="replace").splitlines()
        start = max(1, item["line"] - args.context)

        excerpt = "\n".join(
            f"{n}: {lines[n-1]}"
            for n in range(
                start,
                min(len(lines), item["line"] + args.context) + 1
            )
        )

        if len(excerpt) > remaining:
            excerpt = excerpt[:remaining] + "\n[TRUNCATED]"

        print(excerpt)
        remaining -= len(excerpt)

if args.references:
    for item in matches[:args.limit]:
        print(
            f"REFERENCE: {item['file']}:{item['line']}"
        )

if not (args.definition or args.callers or args.references):
    for item in matches[:args.limit]:
        print(
            f"{item['file']}:{item['line']}: {item['source']}"
        )

print("\nLIMITATIONS:")
print("- Caller detection is lexical and structural, not semantic.")
print("- Macro-generated code may not be visible.")
print("- Complex Rust syntax may require rust-analyzer verification.")
print("- Definitions without brace bodies are not fully extracted.")
print("- Output is bounded; truncated definitions are not complete.")
