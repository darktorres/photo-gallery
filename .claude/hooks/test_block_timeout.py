"""Cases for block_timeout.py. Run from the repository root: python .claude/hooks/test_block_timeout.py"""

import importlib.util
import pathlib

spec = importlib.util.spec_from_file_location("block_timeout", pathlib.Path(__file__).with_name("block_timeout.py"))
assert spec is not None and spec.loader is not None
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)

BLOCKED = [
    "timeout 30 cargo test",
    "cargo build && timeout 5 ./x",
    "echo ok; timeout 10 x",
    "(timeout 5 x)",
    "$(timeout 5 x)",
    "x | timeout 5 y",
    "sudo timeout 5 x",
    "VAR=1 timeout 5 x",
    "C:\\Windows\\System32\\timeout.exe /t 5",
    "/usr/bin/timeout 5 x",
    "& timeout 5 x",
    "timeout",
    "TIMEOUT 5 x",
    "first\ntimeout 5 x",
]

ALLOWED = [
    "cargo test -p pg-metadata -- timeout",
    "git log --grep timeout",
    'echo "timeout 5"',
    "grep -rn timeout src",
    "let timeout_ms = 5;",
    "cargo test timeout_test",
    "python script.py --timeout=30",
    "cat /tmp/timeouts.txt",
    "echo timeouts",
    "timeout_ms=5 cargo test",
]


def blocked(command: str) -> bool:
    return hook.COMMAND_WORD.search(command) is not None


def main() -> None:
    failures = [f"blocked, should be allowed: {c!r}" for c in ALLOWED if blocked(c)]
    failures += [f"allowed, should be blocked: {c!r}" for c in BLOCKED if not blocked(c)]
    for failure in failures:
        print(failure)
    print("ok" if not failures else f"{len(failures)} failures")
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    main()
