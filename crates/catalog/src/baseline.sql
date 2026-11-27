-- The catalog's schema: every table, index, view and trigger, then the rows creation seeds in `meta`. Plain SQL: the
-- shared SQL macros (`sql.rs`) are expanded in it. `bring_to` adds the `path_rules` row, the application id and the
-- version. Before the first release this file is edited in place; after it, it is frozen and each change is a migration.
-- `vacuum_owed` is 0 here: `bring_to` sets it when the file has no incremental auto-vacuum.

-- tables
CREATE TABLE scans(
    id       INTEGER PRIMARY KEY,
    root     BLOB NOT NULL,      -- the folder as stored (Name bytes, PathRules::join)
    started  INTEGER NOT NULL,   -- Unix seconds
    finished INTEGER
);

CREATE TABLE analysis(
    file_id     INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,  -- analyze::IMPORT_VERSION
    size        INTEGER NOT NULL,  -- the file as analyzed
    mtime       INTEGER NOT NULL,
    rotation    INTEGER NOT NULL,  -- files.turns it was decoded with
    kind        INTEGER NOT NULL,  -- files.kind it was analyzed as (media::Kind::code)
    analyzed    INTEGER NOT NULL,  -- Unix seconds
    error       TEXT,              -- set when the file couldn't be read or decoded; the rest is then NULL
    file_hash   BLOB,              -- tier 0a: BLAKE3 of the bytes
    pixel_hash  BLOB,              -- tier 0b: XXH3-128 of the upright decoded pixels
    format      INTEGER,           -- an image's format::Format::code; NULL for a video
    codec       TEXT,              -- a video's codec, as FFmpeg names it; NULL for an image
    orientation INTEGER,
    width       INTEGER,           -- upright, full size
    height      INTEGER,
    pdq_quality INTEGER,
    pdq_dihedral BLOB,             -- tier 1: the 4 orientations' hashes, 32 bytes each (128 bytes per row; the first is pdq's), for pairs
    frames      INTEGER,           -- frames in the file: > 1 is an animated image (or a video)
    -- When it was taken, as wall-clock time (no time zone) in Unix seconds: the frame every date filter and sort
    -- compares in. A camera's own clock is taken as it is; a UTC instant (a video's creation time) is converted to
    -- the local wall clock of this system's zone (`taken_zone`), so it is also kept below, as the instant.
    taken       INTEGER,
    -- The UTC instant `taken` was converted from (a video's creation time), NULL for a camera's wall clock. `taken` is
    -- computed from it again when the system's zone changes (`set_taken_zone`), so no zone is lost at import.
    taken_utc   INTEGER,
    duration_ms INTEGER,           -- videos
    poster_ms   INTEGER,           -- videos: where the frame for the ThumbHash and the thumbnails is
    -- Borders (borders::detect) on the stored pixels, found after the import by a pass of their own: what a crop
    -- cuts from the left, top, right and bottom (detection::Detection::cut), all 0 for none. NULL until looked for,
    -- and where they aren't (videos, animated and tiny images, failures). `borders_due`: to be looked for, by this
    -- build's detector (the import sets it; Catalog::prepare too when meta.borders_version differs).
    cut_left    INTEGER,
    cut_top     INTEGER,
    cut_right   INTEGER,
    cut_bottom  INTEGER,
    borders_due INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE border_sides(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pass     INTEGER NOT NULL,  -- the peeling round (nested borders come later)
    edge     INTEGER NOT NULL CHECK (edge BETWEEN 0 AND 3),   -- detection::Edge::code
    kind     INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 1),   -- detection::Kind::code
    class    INTEGER NOT NULL CHECK (class BETWEEN 0 AND 3),  -- detection::Class::code
    depth    INTEGER NOT NULL,  -- stored pixels from the edge of what earlier rounds left
    color    INTEGER,           -- solid: 0xRRGGBB at the edge
    straight REAL NOT NULL,
    dev      REAL,              -- solid and measured: flatness
    drift    REAL,
    rejected INTEGER,           -- detection::Reason::code; NULL: accepted
    PRIMARY KEY(file_id, pass, edge)
) WITHOUT ROWID;

CREATE TABLE border_marks(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    marked  INTEGER NOT NULL  -- Unix seconds
);

CREATE TABLE pdq_pairs(
    a        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,  -- a < b
    b        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    distance INTEGER NOT NULL,  -- dihedral variant of the newer file that matched (pdq::hash_rgb_dihedral)
    PRIMARY KEY(a, b)
) WITHOUT ROWID;

CREATE TABLE ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    rating  INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated INTEGER NOT NULL  -- Unix seconds
);

CREATE TABLE file_ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    value   INTEGER NOT NULL CHECK (value BETWEEN 1 AND 5)  -- unrated is no row
);

CREATE TABLE views(
    name      TEXT PRIMARY KEY,
    folder    BLOB NOT NULL,     -- the merged folder's parts, joined (PathRules::join_merged); empty = all folders
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);

CREATE TABLE meta(
    key   TEXT PRIMARY KEY,
    value ANY NOT NULL
);

CREATE TABLE notices(
    id        INTEGER PRIMARY KEY,
    uid       INTEGER NOT NULL DEFAULT 0,
    key       TEXT UNIQUE,
    part      TEXT,
    root      INTEGER REFERENCES roots(id) ON DELETE CASCADE,
    level     INTEGER NOT NULL,   -- 0 info, 1 warning, 2 error
    source    TEXT NOT NULL,      -- Source::code
    kind      TEXT NOT NULL,      -- Notice::code
    data      TEXT NOT NULL,      -- the notice, JSON
    count     INTEGER NOT NULL DEFAULT 1,
    created   INTEGER NOT NULL,
    updated   INTEGER NOT NULL,
    read      INTEGER,
    dismissed INTEGER
, rev INTEGER NOT NULL DEFAULT 0);

CREATE TABLE file_failures(
    id         INTEGER PRIMARY KEY,
    file_id    INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pass       TEXT NOT NULL,      -- FilePass::code: 'thumbs' | 'borders' | 'import'
    mtime      INTEGER NOT NULL,   -- the file as it failed
    size       INTEGER NOT NULL,
    rotation   INTEGER NOT NULL,
    error      TEXT NOT NULL,
    retry      INTEGER NOT NULL,
    serial     INTEGER NOT NULL DEFAULT 0,
    first_seen INTEGER NOT NULL, hidden INTEGER NOT NULL DEFAULT 0,   -- Unix seconds
    UNIQUE(file_id, pass)
);

CREATE TABLE ledger_ack(
    kind      TEXT PRIMARY KEY,   -- FilePass::code: 'thumbs' | 'borders' | 'import'
    seen_to   INTEGER NOT NULL,   -- a serial (the import's: files.seq)
    read      INTEGER,
    dismissed_to INTEGER);

CREATE TABLE "session"(
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    folder    BLOB NOT NULL,     -- as views.folder
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);

CREATE TABLE "file_writes"(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
    value    INTEGER NOT NULL,  -- write::Write::stored: a rating's stars, or quarter turns
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       BLOB,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind),
    CHECK (CASE kind WHEN 'rating' THEN value BETWEEN 0 AND 5 AND coalesce(temp_value BETWEEN 0 AND 5, 1)
                ELSE value BETWEEN 0 AND 3 AND coalesce(temp_value BETWEEN 0 AND 3, 1) END)
);

CREATE TABLE folder_failures(
    id         INTEGER PRIMARY KEY,
    root       INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    path       BLOB NOT NULL,          -- exact bytes, as folders.path
    path_key   BLOB NOT NULL UNIQUE,   -- the folder's folders.path_key: one row per folder
    reason     TEXT NOT NULL,          -- 'unreadable' | 'mount-refused' | 'refused'
    error      TEXT NOT NULL,
    walk       INTEGER NOT NULL,       -- the scans row of the walk that last saw it
    serial     INTEGER NOT NULL DEFAULT 0,
    first_seen INTEGER NOT NULL        -- Unix seconds
);

CREATE TABLE scan_leases(
    id        INTEGER PRIMARY KEY,     -- the walk's scans row
    root      BLOB NOT NULL,           -- the folder it walks, as scans.root
    holder    TEXT NOT NULL,           -- who walks it, for the refusal's words
    heartbeat INTEGER NOT NULL         -- Unix seconds: renewed with each batch the walk writes
);

CREATE TABLE pairs_owed(
    file_id INTEGER PRIMARY KEY REFERENCES analysis(file_id) ON DELETE CASCADE
);

CREATE TABLE hashes_owed(
    file_id INTEGER PRIMARY KEY REFERENCES analysis(file_id) ON DELETE CASCADE
);

CREATE TABLE placeholders(
    file_id  INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    size     INTEGER NOT NULL,
    mtime    INTEGER NOT NULL,
    rotation INTEGER NOT NULL,
    version  INTEGER NOT NULL,
    hash     BLOB NOT NULL
);

CREATE TABLE root_removals(
    id       INTEGER PRIMARY KEY,
    path     BLOB NOT NULL UNIQUE,
    started  INTEGER NOT NULL,   -- Unix seconds
    deferred INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE "roots"(id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL UNIQUE, added INTEGER NOT NULL);

-- The library folders whose scan was asked for and is not yet complete (a folder added, or rescanned): a request made
-- before the app quits is walked at the next start. A complete walk of the folder removes its row; a removed folder's row
-- goes with it.
CREATE TABLE scan_requests(root INTEGER PRIMARY KEY REFERENCES roots(id) ON DELETE CASCADE);

-- The temp files of discarded writes that could not be deleted then (a file in use): the discard drops their rows, so
-- these are what remains of them. The app deletes them at its next start, and a deleted one leaves the table.
CREATE TABLE temp_leftovers(path BLOB PRIMARY KEY);

-- The volumes the files are on, interned: a file row holds the id (one byte for a small catalog), not the 64-bit
-- serial. The serial is the volume's identity (pg_model::ids::FileIdentity::volume, stored signed); a row is made the
-- first time a file of the volume is written, and kept. Reads that hand a volume out map the id back to its serial.
CREATE TABLE volumes(id INTEGER PRIMARY KEY, serial INTEGER NOT NULL UNIQUE);

-- One row per merged folder key (pg_model::rules::PathRules::folder_key: drive-free, each part by its name key), so
-- every drive's copy of a folder is one row, and its files interleave by name. The grid orders by (key, the name key,
-- the id). Not AUTOINCREMENT: nothing keeps an id of it, and a row with no folder is deleted with its last folder.
CREATE TABLE mfolders(id INTEGER PRIMARY KEY, key BLOB NOT NULL UNIQUE);

-- One row per folder, found by path_key: the spelling under the path rules (case-folded on Windows, so one folder is one
-- row whatever its case). path is the spelling last seen, for display. mid is its merged folder (mfolders). See
-- pg_model::rules::PathRules::path_key.
CREATE TABLE "folders"(id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL, path_key BLOB NOT NULL UNIQUE, mid INTEGER NOT NULL REFERENCES mfolders(id), UNIQUE(id, mid));

-- A file: its identity (volume: volumes.id), its folder and merged folder (the composite key makes the database keep
-- mid equal to its folder's, and a folder's re-key carries to its files: ON UPDATE CASCADE), its exact name, its
-- extension and kind as codes (media::Ext::code, media::Kind::code).
CREATE TABLE "files"(id INTEGER PRIMARY KEY AUTOINCREMENT, volume INTEGER REFERENCES volumes(id), file_ref INTEGER, created INTEGER, folder_id INTEGER NOT NULL, mid INTEGER NOT NULL, name BLOB NOT NULL, ext INTEGER NOT NULL, kind INTEGER NOT NULL CHECK (kind IN (0, 1)), size INTEGER NOT NULL, mtime INTEGER NOT NULL, missing INTEGER NOT NULL DEFAULT 0, turns INTEGER NOT NULL DEFAULT 0, pending INTEGER NOT NULL DEFAULT 1, seq INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0, unpaired INTEGER NOT NULL DEFAULT 0, UNIQUE(volume, file_ref), FOREIGN KEY(folder_id, mid) REFERENCES folders(id, mid) ON UPDATE CASCADE);

-- indexes
-- The hash indexes hold only the hashed rows: a row with no hash (not hashed yet, a video's pixel hash) is never looked
-- up by it, and an `x = ?` term implies `x IS NOT NULL`, so the lookups use them as they are.
CREATE INDEX analysis_file_hash ON analysis(file_hash) WHERE file_hash IS NOT NULL;

CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash) WHERE pixel_hash IS NOT NULL;

CREATE INDEX analysis_failed ON analysis(file_id) WHERE error IS NOT NULL;

CREATE INDEX analysis_borders_due ON analysis(file_id) WHERE borders_due = 1;

CREATE INDEX border_sides_class ON border_sides(class, file_id) WHERE rejected IS NULL;

CREATE INDEX pdq_pairs_b ON pdq_pairs(b);

CREATE INDEX notices_live ON notices(dismissed, updated DESC);

CREATE INDEX notices_root ON notices(root);

CREATE INDEX file_failures_serial ON file_failures(pass, serial);

CREATE INDEX file_writes_next ON file_writes((temp IS NULL), queued);

CREATE INDEX notices_kind_root ON notices(kind, root);

CREATE INDEX file_failures_live ON file_failures(pass, serial) WHERE hidden = 0;

CREATE INDEX folder_failures_serial ON folder_failures(serial);

CREATE INDEX folder_failures_root ON folder_failures(root);

CREATE INDEX folders_mid ON folders(mid);

-- The grid's order (both directions) and the name lookups: a merged folder's files by their name key (the registered
-- function path_name_key, the rules' name key), present and missing apart.
CREATE INDEX files_mn ON files(mid, missing, path_name_key(name));

-- A folder's files, present and missing apart; its leading (folder_id, mid) is the composite foreign key's child index.
CREATE INDEX files_fm ON files(folder_id, mid, missing);

CREATE INDEX files_seq ON files(seq);

CREATE INDEX files_to_import ON files(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 1;

-- views
CREATE VIEW thumb_clock AS SELECT value AS seq FROM meta WHERE key = 'seq';

-- volume is the serial (a present file has one: a displaced row is missing). A filter on it seeks the volume by its
-- serial, then its files by the identity index; `+volume` leaves the planner on another index (`files_seq`).
CREATE VIEW thumb_sources AS
    SELECT f.id, v.serial AS volume, f.file_ref, f.folder_id,
           (SELECT path FROM folders WHERE id = f.folder_id) AS folder,
           f.name, f.mtime, f.turns, f.size, f.seq,
           CASE WHEN f.kind = 1 THEN coalesce((SELECT poster_ms FROM analysis WHERE file_id = f.id), 0)
           END AS poster_ms
    FROM files f JOIN volumes v ON v.id = f.volume
    WHERE f.missing = 0
      AND EXISTS (SELECT 1 FROM analysis WHERE file_id = f.id AND error IS NULL AND mtime = f.mtime);

-- The thumbnail cache reads the catalog through these views only (`pg_thumbcache`'s reader, with `thumb_clock` and
-- `thumb_sources` above): the id high-water mark (the ids are never reissued), the ids that exist, the volumes the
-- catalog holds files on (their serials: one probe of the identity index a volume), and the thumbnail failures.
CREATE VIEW thumb_high_water AS SELECT coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'files'), 0) AS high;

CREATE VIEW thumb_file_ids AS SELECT id FROM files;

CREATE VIEW thumb_volumes AS
    SELECT v.serial AS volume FROM volumes v WHERE EXISTS (SELECT 1 FROM files f WHERE f.volume = v.id);

CREATE VIEW thumb_failures AS SELECT file_id, pass, mtime, size, rotation, retry FROM file_failures;

CREATE VIEW present_pdq_pairs AS
    SELECT p.* FROM pdq_pairs p JOIN files fa ON fa.id = p.a JOIN files fb ON fb.id = p.b
    WHERE fa.missing = 0 AND fb.missing = 0;

-- triggers
CREATE TRIGGER analysis_changed AFTER UPDATE OF version, size, mtime, rotation, kind, error, poster_ms ON analysis
WHEN old.version IS NOT new.version OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
    OR old.rotation IS NOT new.rotation OR old.kind IS NOT new.kind OR old.error IS NOT new.error
    OR old.poster_ms IS NOT new.poster_ms
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET pending = NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.turns AND a.kind = files.kind), seq = (SELECT value FROM meta WHERE key = 'seq'), changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER analysis_removed AFTER DELETE ON analysis BEGIN
    UPDATE files SET pending = 1 WHERE id = old.file_id;
END;

CREATE TRIGGER ratings_stamp_insert AFTER INSERT ON ratings BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER ratings_stamp_update AFTER UPDATE ON ratings BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER ratings_stamp_delete AFTER DELETE ON ratings BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = old.file_id;
END;

CREATE TRIGGER border_marks_stamp_insert AFTER INSERT ON border_marks BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER border_marks_stamp_delete AFTER DELETE ON border_marks BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = old.file_id;
END;

CREATE TRIGGER analysis_removed_stamp AFTER DELETE ON analysis BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = old.file_id;
END;

CREATE TRIGGER border_sides_stamp_insert AFTER INSERT ON border_sides BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER border_sides_stamp_delete AFTER DELETE ON border_sides BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = old.file_id;
END;

CREATE TRIGGER file_failures_stamp AFTER INSERT ON file_failures BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE file_failures SET serial = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.id;
END;

CREATE TRIGGER file_writes_stamp_insert AFTER INSERT ON file_writes BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER file_writes_stamp_update AFTER UPDATE ON file_writes BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER file_writes_stamp_delete AFTER DELETE ON file_writes BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = old.file_id;
END;

CREATE TRIGGER notices_uid AFTER INSERT ON notices BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE notices SET uid = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.id;
    DELETE FROM notices WHERE new.key IS NULL AND id IN (
        SELECT id FROM notices WHERE key IS NULL AND id <> new.id ORDER BY read IS NULL, uid
        LIMIT max(0, (SELECT count(*) FROM notices WHERE key IS NULL) - 1000));
END;

CREATE TRIGGER analysis_import_rewritten AFTER UPDATE OF size, mtime, rotation ON analysis WHEN new.error IS NOT NULL
BEGIN
    UPDATE file_failures SET size = new.size, mtime = new.mtime, rotation = new.rotation
    WHERE file_id = new.file_id AND pass = 'import';
END;

CREATE TRIGGER analysis_import_removed AFTER DELETE ON analysis BEGIN
    DELETE FROM file_failures WHERE file_id = old.file_id AND pass = 'import';
END;

CREATE TRIGGER folder_failures_stamp AFTER INSERT ON folder_failures BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE folder_failures SET serial = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.id;
END;

CREATE TRIGGER analysis_import AFTER INSERT ON analysis
WHEN new.error IS NOT NULL OR EXISTS (SELECT 1 FROM file_failures WHERE file_id = new.file_id AND pass = 'import')
BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (new.error IS NULL OR mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed,
               (f.missing OR f.pending)
        FROM files f WHERE new.error IS NOT NULL AND f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error, hidden = excluded.hidden
        WHERE file_failures.error IS NOT excluded.error OR file_failures.hidden IS NOT excluded.hidden;
END;

CREATE TRIGGER analysis_pairing_added AFTER INSERT ON analysis BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'pairing_epoch';
END;

CREATE TRIGGER analysis_pairing_removed AFTER DELETE ON analysis WHEN old.pdq_dihedral IS NOT NULL BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'pairing_epoch';
END;

CREATE TRIGGER analysis_pairing_changed AFTER UPDATE OF file_id, pdq_dihedral, pdq_quality ON analysis
WHEN old.file_id IS NOT new.file_id OR old.pdq_dihedral IS NOT new.pdq_dihedral OR old.pdq_quality IS NOT new.pdq_quality
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'pairing_epoch';
END;

CREATE TRIGGER placeholders_stamp_insert AFTER INSERT ON placeholders BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER placeholders_stamp_update AFTER UPDATE OF hash, size, mtime, rotation, version ON placeholders
WHEN old.hash IS NOT new.hash OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
    OR old.rotation IS NOT new.rotation OR old.version IS NOT new.version
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER placeholders_stamp_delete AFTER DELETE ON placeholders BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = old.file_id;
END;

CREATE TRIGGER analysis_cut_stamp AFTER UPDATE OF cut_left, cut_top, cut_right, cut_bottom, width, height ON analysis
WHEN old.cut_left IS NOT new.cut_left OR old.cut_top IS NOT new.cut_top OR old.cut_right IS NOT new.cut_right
    OR old.cut_bottom IS NOT new.cut_bottom OR old.width IS NOT new.width OR old.height IS NOT new.height
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

CREATE TRIGGER files_pending_changed AFTER UPDATE OF size, mtime, turns, kind ON files
WHEN old.size IS NOT new.size OR old.mtime IS NOT new.mtime OR old.turns IS NOT new.turns
    OR old.kind IS NOT new.kind
BEGIN
    UPDATE files SET pending = NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.turns AND a.kind = files.kind) WHERE id = new.id;
END;

CREATE TRIGGER file_failures_hidden AFTER UPDATE OF pending, missing ON files
WHEN old.pending IS NOT new.pending OR old.missing IS NOT new.missing
BEGIN
    UPDATE file_failures SET hidden = (new.missing OR (pass = 'import' AND new.pending))
    WHERE file_id = new.id AND hidden IS NOT (new.missing OR (pass = 'import' AND new.pending));
END;

CREATE TRIGGER file_failures_stale AFTER UPDATE OF mtime, size, turns ON files
WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.turns IS NOT new.turns
BEGIN
    DELETE FROM file_failures WHERE file_id = new.id AND pass <> 'import';
END;

CREATE TRIGGER files_source_changed AFTER UPDATE OF mtime, turns, missing, size ON files
WHEN old.mtime IS NOT new.mtime OR old.turns IS NOT new.turns OR old.missing IS NOT new.missing
    OR old.size IS NOT new.size
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET seq = (SELECT value FROM meta WHERE key = 'seq'), changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.id;
END;

-- seeds
INSERT INTO meta(key, value) VALUES ('borders_version', 0);
INSERT INTO meta(key, value) VALUES ('catalog_id', lower(hex(randomblob(16))));
INSERT INTO meta(key, value) VALUES ('held_exts', '');
INSERT INTO meta(key, value) VALUES ('import_version', 0);
INSERT INTO meta(key, value) VALUES ('pairing_epoch', 0);
INSERT INTO meta(key, value) VALUES ('pairing_version', 1);
INSERT INTO meta(key, value) VALUES ('seq', 0);
-- How many Retries of a pass's failed files have been made (`relist_failures`): a pass's failures found before one are
-- not recorded after it (`Catalog::record_failures_since`).
INSERT INTO meta(key, value) VALUES ('failure_retries', 0);
INSERT INTO meta(key, value) VALUES ('taken_zone', '');
INSERT INTO meta(key, value) VALUES ('thumbs_version', 0);
INSERT INTO meta(key, value) VALUES ('vacuum_owed', 0);
INSERT INTO meta(key, value) VALUES ('written_by', '');
