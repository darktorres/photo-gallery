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
    rotation    INTEGER NOT NULL,  -- files.rotation it was decoded with
    kind        TEXT NOT NULL,     -- files.kind it was analyzed as
    analyzed    INTEGER NOT NULL,  -- Unix seconds
    error       TEXT,              -- set when the file couldn't be read or decoded; the rest is then NULL
    file_hash   BLOB,              -- tier 0a: BLAKE3 of the bytes
    pixel_hash  BLOB,              -- tier 0b: XXH3-128 of the upright decoded pixels
    format      TEXT,
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
    edge     TEXT NOT NULL CHECK (edge IN ('top', 'bottom', 'left', 'right')),
    kind     TEXT NOT NULL CHECK (kind IN ('solid', 'blur')),
    class    TEXT NOT NULL CHECK (class IN ('black', 'white', 'color', 'blur')),
    depth    INTEGER NOT NULL,  -- stored pixels from the edge of what earlier rounds left
    color    INTEGER,           -- solid: 0xRRGGBB at the edge
    straight REAL NOT NULL,
    dev      REAL,              -- solid and measured: flatness
    drift    REAL,
    rejected TEXT,              -- detection::Reason key; NULL: accepted
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
    value   INTEGER NOT NULL CHECK (value BETWEEN 0 AND 5)
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
    path       BLOB NOT NULL UNIQUE,   -- exact bytes, as folders.path
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

CREATE TABLE "folders"(id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL UNIQUE, key BLOB NOT NULL);

CREATE TABLE "files"(id INTEGER PRIMARY KEY AUTOINCREMENT, volume INTEGER, file_ref INTEGER, created INTEGER, folder_id INTEGER NOT NULL REFERENCES folders(id), name BLOB NOT NULL, ext TEXT NOT NULL, kind TEXT NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL, missing INTEGER NOT NULL DEFAULT 0, rotation INTEGER NOT NULL DEFAULT 0, order_key BLOB NOT NULL, pending INTEGER NOT NULL DEFAULT 1, seq INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0, unpaired INTEGER NOT NULL DEFAULT 0, UNIQUE(volume, file_ref));

-- indexes
CREATE INDEX analysis_file_hash ON analysis(file_hash);

CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash);

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

CREATE INDEX folders_key ON folders(key);

CREATE INDEX files_changed ON files(changed);

CREATE INDEX files_folder ON files(folder_id, name);

CREATE INDEX files_order ON files(order_key) WHERE missing = 0;

CREATE INDEX files_present ON files(folder_id) WHERE missing = 0;

CREATE INDEX files_ref ON files(file_ref);

CREATE INDEX files_seq ON files(seq);

CREATE INDEX files_to_import ON files(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 1;

-- views
CREATE VIEW thumb_clock AS SELECT value AS seq FROM meta WHERE key = 'seq';

CREATE VIEW thumb_sources AS
    SELECT f.id, f.volume, f.file_ref, f.folder_id,
           (SELECT path FROM folders WHERE id = f.folder_id) AS folder,
           f.name, f.mtime, f.rotation, f.size, f.seq,
           CASE WHEN f.kind = 'video' THEN coalesce((SELECT poster_ms FROM analysis WHERE file_id = f.id), 0)
           END AS poster_ms
    FROM files f
    WHERE f.missing = 0
      AND EXISTS (SELECT 1 FROM analysis WHERE file_id = f.id AND error IS NULL AND mtime = f.mtime);

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
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.rotation AND a.kind = files.kind), seq = (SELECT value FROM meta WHERE key = 'seq'), changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
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

CREATE TRIGGER files_pending_changed AFTER UPDATE OF size, mtime, rotation, kind ON files
WHEN old.size IS NOT new.size OR old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation
    OR old.kind IS NOT new.kind
BEGIN
    UPDATE files SET pending = NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.rotation AND a.kind = files.kind) WHERE id = new.id;
END;

CREATE TRIGGER file_failures_hidden AFTER UPDATE OF pending, missing ON files
WHEN old.pending IS NOT new.pending OR old.missing IS NOT new.missing
BEGIN
    UPDATE file_failures SET hidden = (new.missing OR (pass = 'import' AND new.pending))
    WHERE file_id = new.id AND hidden IS NOT (new.missing OR (pass = 'import' AND new.pending));
END;

CREATE TRIGGER file_failures_stale AFTER UPDATE OF mtime, size, rotation ON files
WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation
BEGIN
    DELETE FROM file_failures WHERE file_id = new.id AND pass <> 'import';
END;

CREATE TRIGGER files_source_changed AFTER UPDATE OF mtime, rotation, missing, size ON files
WHEN old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation OR old.missing IS NOT new.missing
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
INSERT INTO meta(key, value) VALUES ('taken_zone', '');
INSERT INTO meta(key, value) VALUES ('thumbs_version', 0);
INSERT INTO meta(key, value) VALUES ('vacuum_owed', 0);
INSERT INTO meta(key, value) VALUES ('written_by', '');
