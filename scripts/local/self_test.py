import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parent
libraries = {
    "common.py", "trading.py", "repository.py",
    "testing.py", "orchestration.py", "monitoring.py",
    "self_test.py"
}
passed = failed = 0

for p in sorted(root.glob("*.py")):
    try:
        compile(p.read_text(), str(p), "exec")
        if p.name not in libraries:
            result = subprocess.run(
                [sys.executable,str(p),"--help"],
                capture_output=True,text=True,timeout=10
            )
            if result.returncode:
                raise RuntimeError(result.stderr[:500])
        print("PASS",p.name)
        passed += 1
    except Exception as e:
        print("FAIL",p.name,str(e))
        failed += 1

print(f"\nPassed: {passed}; Failed: {failed}")
sys.exit(1 if failed else 0)
