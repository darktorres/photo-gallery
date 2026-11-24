#!/usr/bin/env python3
"""PreToolUse hook for Bash and PowerShell: blocks a command that runs the `timeout` program.

Only the program counts: `timeout` as a command word, at the start of a command, after a separator (`;` `&` `|` `(`
`{` a backtick, a newline or `$(`), or after a wrapper (`sudo`, `env`, `exec`, `nohup`, `time`, `xargs`, `then`, `do`,
`else`, or `NAME=value`). It may be path-qualified, with or without `.exe`, and any case. The word anywhere else is
allowed: as an argument, a flag, in quotes, or inside a name such as `timeout_ms` or `timeouts.txt`.

The tool's own timeout parameter bounds a command here (the owner's rule). The hook reads the tool call as JSON on
stdin; exit status 2 blocks the call and shows the message to the model. Input that isn't a shell call with a command
is allowed, so the tool reports the malformed call itself.
"""

import json
import re
import sys

COMMAND_WORD = re.compile(
    r"""
    (?:^|[;&|(){}`\n]|\$\()                                # the start of a command: the string's start, a separator, or $(
    \s*
    (?:(?:\w+=\S*|sudo|env|exec|nohup|time|xargs|then|do|else)\s+)*   # wrappers and NAME=value assignments
    (?:\S*[/\\])?                                          # a path to the program
    timeout(?:\.exe)?                                      # the program
    (?=\s|$)                                               # a word of its own: arguments follow, or nothing
    """,
    re.IGNORECASE | re.VERBOSE | re.MULTILINE,
)

MESSAGE = (
    "Blocked: do not run the `timeout` program in a command. Run the command without it, and bound it with the tool's "
    "timeout parameter instead (up to 600000 ms)."
)


def main() -> None:
    try:
        payload = json.load(sys.stdin)
        command = payload["tool_input"]["command"]
    except (ValueError, KeyError, TypeError):
        return
    if not isinstance(command, str):
        return
    if COMMAND_WORD.search(command):
        print(MESSAGE, file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
