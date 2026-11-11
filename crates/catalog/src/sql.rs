//! The catalog's SQL macros. A macro is a fragment that more than one statement uses, so every statement stays a
//! constant (`concat!`), none made at run time. `lib.rs` declares this module first, with `#[macro_use]`: `macro_rules!`
//! is textually scoped, so the modules declared after it can use these.

/// Whether the `files` row being written needs its analysis (re)done: no analysis made from it as it is now (at
/// `meta.import_version` or later, from the same size, mtime, catalog rotation and kind). A failure counts as made:
/// `--retry` redoes those. Inside `UPDATE files … WHERE id = …`.
macro_rules! stale {
    () => {
        "NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.rotation AND a.kind = files.kind)"
    };
}

/// The next number of the change sequence, for `files.seq`: `next_seq!()` first, then `seq = seq!()`.
macro_rules! next_seq {
    () => {
        "UPDATE meta SET value = value + 1 WHERE key = 'seq'"
    };
}
/// A trigger that stamps a file's `changed` (`files.changed`, the next `seq`) when a row of `table` that names it
/// (by `file_id`, of the `new` or `old` row) changes: `name`, after `event`, optionally only `when` the condition holds.
/// The text is the one the triggers were written with, piece by piece.
macro_rules! stamp {
    ($name:literal, $event:literal, $table:literal, $row:ident) => {
        concat!(
            "CREATE TRIGGER ",
            $name,
            " AFTER ",
            $event,
            " ON ",
            $table,
            " BEGIN\n    ",
            next_seq!(),
            ";\n    UPDATE files SET changed = ",
            seq!(),
            " WHERE id = ",
            stringify!($row),
            ".file_id;\nEND;\n"
        )
    };
    ($name:literal, $event:literal, $table:literal, $row:ident, when $when:literal) => {
        concat!(
            "CREATE TRIGGER ",
            $name,
            " AFTER ",
            $event,
            " ON ",
            $table,
            "\nWHEN ",
            $when,
            "\nBEGIN\n    ",
            next_seq!(),
            ";\n    UPDATE files SET changed = ",
            seq!(),
            " WHERE id = ",
            stringify!($row),
            ".file_id;\nEND;\n"
        )
    };
}
macro_rules! seq {
    () => {
        "(SELECT value FROM meta WHERE key = 'seq')"
    };
}

/// The files of a pass's failure ledger (`?1`, the pass's code), `$which` narrowing the rows, are listed again by the
/// pass: their `seq` moves on to the number `next_seq!()` took, past the pass's checkpoint (`files.seq` triggers neither
/// the ledger's trigger nor `files_source_changed`). The rows are then deleted by the caller.
macro_rules! failures_relisted {
    ($which:literal) => {
        concat!(
            "UPDATE files SET seq = ",
            seq!(),
            " WHERE id IN (SELECT file_id FROM file_failures WHERE pass = ?1",
            $which,
            ")"
        )
    };
}

/// The analyses borders are looked for in: still images that decoded and are big enough (detection::MIN_SIDE).
macro_rules! has_borders {
    () => {
        concat!(
            "error IS NULL AND kind = '",
            pg_model::kind_image!(),
            "' AND frames <= 1 AND min(width, height) >= ",
            pg_model::min_side!()
        )
    };
}

/// Folders equal to the root (?1) or below it: the paths from ?2 (the root and a separator) up to ?3 (the root and the
/// separator's successor, so nothing sorts past the subtree). The bounds are made by
/// `under_params`.
macro_rules! under_root {
    () => {
        "SELECT id FROM folders WHERE path = ?1 OR (path >= ?2 AND path < ?3)"
    };
}

/// The data a row can hold that a pairing would lose (S-17.7 decision 4): a rotation, a rating, the file's own rating,
/// an analysis or a queued write. Inside a statement over `files`.
macro_rules! has_children {
    () => {
        "(files.rotation <> 0 OR EXISTS (SELECT 1 FROM ratings WHERE file_id = files.id)
          OR EXISTS (SELECT 1 FROM file_ratings WHERE file_id = files.id)
          OR EXISTS (SELECT 1 FROM analysis WHERE file_id = files.id)
          OR EXISTS (SELECT 1 FROM file_writes WHERE file_id = files.id))"
    };
}

/// What a `WorkItem` is read from, with `f` the file: its folder's id and its name, which
/// `work_list` makes its path from.
macro_rules! work_columns {
    () => {
        "f.id, f.folder_id, f.name, f.size, f.mtime, f.rotation, f.kind"
    };
}

/// `Catalog::pending_import`'s query, with the volume and the record number for its order.
macro_rules! pending_files {
    () => {
        // `missing = 0 AND pending = 1` written out: the index's condition.
        concat!(
            "SELECT ",
            work_columns!(),
            ", f.volume AS volume, f.file_ref & 0xFFFFFFFFFFFF AS record
             FROM files f WHERE f.missing = 0 AND f.pending = 1"
        )
    };
}

/// No write of the app's into file `?1` (or, as `not_being_written!(files)`, the `files` row written) is in progress (`file_writes.temp`, set once the complete new content is
/// beside the file and the file is open for the copy, before the file is rewritten in place, and cleared in the
/// transaction that records the result; a copy that keeps failing keeps it): what was read from it is what the
/// catalog says, not a part-written file.
macro_rules! not_being_written {
    () => {
        "NOT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND temp IS NOT NULL)"
    };
    // The same rule for the `files` row a statement is writing (the scan's upsert, whose `?1` is the volume).
    (files) => {
        "NOT EXISTS(SELECT 1 FROM file_writes WHERE file_id = files.id AND temp IS NOT NULL)"
    };
}

/// The most Events (keyless notices) the inbox keeps: past it, the `notices_uid` trigger deletes the oldest, read ones
/// first, at each insert (`TO_8_NOTIFY`). One source for the trigger's text and `inbox::KEPT_EVENTS`.
macro_rules! kept_events {
    () => {
        1000
    };
}

/// The ledger's rows the inbox shows: those of files that are present, and for the import also not pending
/// (`file_failures.hidden`, kept by `files_failures_hidden`). Written out as the literal the partial index
/// `file_failures_live` is made with, since SQLite uses a partial index only when the query repeats its condition.
macro_rules! live_failures {
    () => {
        "hidden = 0"
    };
}
