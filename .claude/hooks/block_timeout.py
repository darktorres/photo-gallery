#!/usr/bin/env python3
"""PreToolUse hook for Bash and PowerShell: blocks a command that uses `timeout`.

The tool's own timeout parameter bounds a command here (the owner's rule), so `timeout` is not used in a command line.
The hook reads the tool call as JSON on stdin; exit status 2 blocks the call and shows the message to the model.
"""

import json
import re
import sys

TIMEOUT = re.compile(r"\btimeout\b")
MESSAGE = (
    "Blocked: do not use `timeout` in a command. Run the command without it, and bound it with the tool's timeout "
    "parameter instead (up to 600000 ms)."
)


def main() -> None:
    payload = json.load(sys.stdin)
    command = payload.get("tool_input", {}).get("command", "")
    if TIMEOUT.search(command):
        print(MESSAGE, file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
