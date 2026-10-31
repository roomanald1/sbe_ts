#!/usr/bin/env python3
"""Rebuild/check the read-only eth_call helper with the pinned official solc."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

folder = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("solc", type=Path, help="Official solc 0.8.30 Linux binary")
parser.add_argument("--write", action="store_true", help="Replace checked-in runtime after review")
args = parser.parse_args()
build = json.loads((folder / "paper_liquidator_build.json").read_text())
compiler_hash = hashlib.sha256(args.solc.read_bytes()).hexdigest()
if compiler_hash != build["compiler_sha256"].removeprefix("0x"):
    raise SystemExit("Compiler checksum differs from pinned official solc")
source = (folder / "PaperLiquidator.sol").read_text()
request = {
    "language": "Solidity",
    "sources": {"PaperLiquidator.sol": {"content": source}},
    "settings": {
        "optimizer": {"enabled": True, "runs": 200},
        "viaIR": True,
        "evmVersion": "paris",
        "outputSelection": {"*": {"*": ["evm.deployedBytecode.object", "evm.methodIdentifiers"]}},
    },
}
process = subprocess.run([str(args.solc.resolve()), "--standard-json"], input=json.dumps(request),
                         text=True, capture_output=True, check=True)
result = json.loads(process.stdout)
errors = [e for e in result.get("errors", []) if e["severity"] == "error"]
if errors:
    raise SystemExit(json.dumps(errors, indent=2))
contract = result["contracts"]["PaperLiquidator.sol"]["PaperLiquidator"]["evm"]
runtime = contract["deployedBytecode"]["object"]
source_hash = hashlib.sha256(source.encode()).hexdigest()
runtime_hash = hashlib.sha256(bytes.fromhex(runtime)).hexdigest()
if args.write:
    (folder / "paper_liquidator_runtime.hex").write_text(runtime + "\n")
    build.update(source_sha256=source_hash, runtime_sha256=runtime_hash,
                 method_identifiers=contract["methodIdentifiers"], optimizer_runs=200, via_ir=True)
    (folder / "paper_liquidator_build.json").write_text(json.dumps(build, indent=2) + "\n")
else:
    assert runtime == (folder / "paper_liquidator_runtime.hex").read_text().strip(), "Runtime drift"
    assert source_hash == build["source_sha256"], "Source hash drift"
    assert runtime_hash == build["runtime_sha256"], "Runtime checksum drift"
    assert contract["methodIdentifiers"] == build["method_identifiers"], "ABI drift"
print("Verified pinned compiler, source, ABI and", len(runtime) // 2, "bytes of runtime")
