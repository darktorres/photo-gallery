"""Where a Rust file's test section starts, for the module-graph tools.

Every tool here cuts a file the same way: at its first line-start `#[cfg(test)]`, since the test section comes last in a
file by convention. This module is the one place that rule is written. It has no side effects, so a tool can import it
without running another tool (`edges.py` runs its report when it is run, and must not run on an import).

The three report tools (`dupes.py`, `edges.py`, `api_surface.py`) are reports for a review (`just modgraph-report`), not a
gate: each matches by name, so it can both miss a use and invent one. Their docstrings say so in one line that points
here, rather than each repeating the reason.
"""

import re

_CUT = re.compile(r"^#\[cfg\(test\)\]", re.MULTILINE)


def production_part(text: str) -> str:
    """The production code of a Rust file's text: everything before its first line-start `#[cfg(test)]`. The whole text
    when there is none."""
    cut = _CUT.search(text)
    return text[: cut.start()] if cut else text
