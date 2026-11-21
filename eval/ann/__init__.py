"""ANN bake-off on real embeddings: the exact GPU self-join vs tuned approximate methods.
Entry point: python -m ann.run (see run.py)."""

import sys
from pathlib import Path

# eval/ holds common (and edits, group, ...), which every module here imports. This package's __init__ runs before any of its
# modules, so inserting eval/ once here makes the imports work however the package is started.
_EVAL = str(Path(__file__).resolve().parents[1])
if _EVAL not in sys.path:
    sys.path.insert(0, _EVAL)
