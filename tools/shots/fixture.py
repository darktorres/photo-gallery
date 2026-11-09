"""The fixture for the screenshot matrix (`just shots`, tools/compare_shots.py): the parts the hooks need that the
command line can't make.

    python tools/shots/fixture.py library OUT        a small synthetic library: OUT/folder-00 and OUT/folder-01
    python tools/shots/fixture.py views CATALOG      a saved view "alpha" on folder-00 (recursive, by name, descending)
                                                     and the session's folder folder-01, in CATALOG (catalog.sqlite)
    python tools/shots/fixture.py settled CATALOG    fails if the catalog has background work left

The view's folder differs from the session's on purpose: `--apply-view=alpha` must show folder-00.
"""

import argparse
import json
import sqlite3
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def library(out: Path) -> None:
    subprocess.run(
        [
            sys.executable,
            str(REPO / "tools" / "profiling" / "synthetic_library.py"),
            str(out),
            "--folders",
            "2",
            "--per-folder",
            "12",
            "--seed",
            "1",
            "--jobs",
            "4",
            # Small JPEGs: the viewer decodes the original each time it opens a file, and a 12 MP decode is slower
            # than the screenshot's wait, so the default mix (up to 4032x3024) would make the screenshots race.
            "--jpeg-size",
            "1920x1080,1080x1920",
        ],
        check=True,
    )


def merged(path: str) -> str:
    # The stored folder is the merged path: the drive letter stripped (pg_model path.rs merged_path).
    return path.split(":", 1)[1].lstrip("\\") if ":" in path else path


def text_of(stored: bytes | str) -> str:
    # A folder's path is a BLOB of its name bytes (WTF-8 on Windows, S-17.3); a name with no unpaired surrogate is
    # plain UTF-8, which is all this fixture makes.
    return stored if isinstance(stored, str) else stored.decode("utf-8", "surrogatepass")


def folder_path(conn: sqlite3.Connection, name: str) -> str:
    stored = [text_of(p) for (p,) in conn.execute("select path from folders")]
    matches = [p for p in stored if p.replace("/", "\\").endswith(name)]
    if len(matches) != 1:
        sys.exit(f"expected one folder ending in {name}, found {matches}; import the library first")
    return merged(matches[0])


def merged_parts_json(merged_text: str) -> str:
    # The session's folder is the JSON of the folder's parts (pg_model's Merged: an array of names), not its text.
    return json.dumps([part for part in merged_text.split("\\") if part], separators=(",", ":"))


def typed_folders(conn: sqlite3.Connection) -> bool:
    # The layout S-17.8 made: a saved view's folder is a BLOB (the stored bytes of its parts), and the session's
    # folder is the JSON of its parts. An older catalog keeps both as text (the same fixture serves both builds).
    columns = conn.execute("pragma table_info(views)").fetchall()
    return any(name == "folder" and kind.upper() == "BLOB" for _, name, kind, *_ in columns)


def views(catalog: Path) -> None:
    conn = sqlite3.connect(catalog)
    alpha = folder_path(conn, "folder-00")
    beta = folder_path(conn, "folder-01")
    typed = typed_folders(conn)
    view_folder = alpha.encode("utf-8") if typed else alpha
    session_folder = merged_parts_json(beta) if typed else beta
    conn.execute(
        "insert or replace into views(name, folder, recursive, query, sort) values (?, ?, ?, ?, ?)",
        ("alpha", view_folder, 1, "{}", '{"key":"name","descending":true}'),
    )
    session = [
        ("folder", session_folder),
        ("recursive", "false"),
        ("query", "{}"),
        ("sort", '{"key":"name","descending":false}'),
    ]
    for key, value in session:
        conn.execute("insert or replace into session(key, value) values (?, ?)", (key, value))
    conn.commit()
    conn.close()


def settled(catalog: Path) -> None:
    """Exit with an error when the catalog still has background work the app would do at startup."""
    conn = sqlite3.connect(catalog)
    due = conn.execute("select count(*) from analysis where borders_due = 1").fetchone()[0]
    writes = conn.execute("select count(*) from file_writes").fetchone()[0]
    conn.close()
    print(f"borders due: {due}, pending file writes: {writes}")
    if due or writes:
        sys.exit("the catalog still has work: borders due or file writes pending")


def main() -> None:
    parser = argparse.ArgumentParser(description="The screenshot matrix's fixture (see the module docstring).")
    commands = parser.add_subparsers(dest="command", required=True)
    lib = commands.add_parser("library", help="write the synthetic library")
    lib.add_argument("out", type=Path)
    view = commands.add_parser("views", help="add the saved view and the session folder to a catalog")
    view.add_argument("catalog", type=Path)
    check = commands.add_parser("settled", help="fail if the catalog has background work left")
    check.add_argument("catalog", type=Path)
    args = parser.parse_args()
    if args.command == "library":
        library(args.out)
    elif args.command == "views":
        views(args.catalog)
    else:
        settled(args.catalog)


if __name__ == "__main__":
    main()
