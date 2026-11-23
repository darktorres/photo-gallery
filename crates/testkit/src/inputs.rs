//! The names a test or a probe reads from its environment: the variables a test binary's parent sets for a child process,
//! and the inputs an example or an ignored test takes from the person who runs it. They are not the app's variables
//! (those are registered in pg-platform's `env` module and documented in `.env.example`), and production code never
//! reads them. Each constant's value is the variable's name as it is spelled in the environment: the PowerShell probe
//! scripts (`tools/profiling/pairing/*.ps1`) and the Python tools set the probe variables by those names, so the
//! values keep the literal spelling. A name is written once here and used through its constant, so a typo is a compile
//! error, and `tools/test_ci_files.py` reads this file for the names the sources may use.

/// A child process's handshake key (the catalog and dbfile holders): the database file the holder opens.
pub const HOLDER_PATH: &str = "PG_HOLDER_PATH";
/// A child process's handshake key (the catalog and dbfile holders): the mode the holder opens the file in.
pub const HOLDER_MODE: &str = "PG_HOLDER_MODE";
/// A child process's handshake key (the catalog and dbfile holders): the file the holder writes once it holds the lock.
pub const HOLDER_READY: &str = "PG_HOLDER_READY";
/// A child process's handshake key (the catalog and dbfile holders): the file whose appearance releases the holder.
pub const HOLDER_RELEASE: &str = "PG_HOLDER_RELEASE";

/// The thumbnail cache's child-process key: the folder the holder opens the cache in.
pub const THUMB_HOLDER: &str = "PG_THUMB_HOLDER";
/// The thumbnail cache's child-process key: the catalog the holder names.
pub const THUMB_CATALOG: &str = "PG_THUMB_CATALOG";
/// The thumbnail cache's child-process key: the file the holder writes once it holds the cache.
pub const THUMB_READY: &str = "PG_THUMB_READY";
/// The thumbnail cache's child-process key: the file whose appearance releases the holder.
pub const THUMB_RELEASE: &str = "PG_THUMB_RELEASE";

/// A probe input, not the app's (the examples' probe inputs): a command run between a scan probe's two walks, split on
/// spaces (one that empties the file cache, for a cold rescan).
pub const PROBE_BETWEEN: &str = "PG_PROBE_BETWEEN";
/// A probe input, not the app's (the examples' probe inputs): set, the scan probe walks the folder a second time.
pub const PROBE_AGAIN: &str = "PG_PROBE_AGAIN";
/// A probe input, not the app's (the examples' probe inputs): the owed pictures per write transaction.
pub const PROBE_WRITE: &str = "PG_PROBE_WRITE";
/// A probe input, not the app's (the examples' probe inputs): the join's worker threads.
pub const PROBE_WORKERS: &str = "PG_PROBE_WORKERS";
/// A probe input, not the app's (the examples' probe inputs): the join's head words per stage, as comma-separated counts.
pub const PROBE_WIDTHS: &str = "PG_PROBE_WIDTHS";
/// A probe input, not the app's (the examples' probe inputs): the pairing pass's slice size.
pub const PROBE_SLICE: &str = "PG_PROBE_SLICE";
/// A probe input, not the app's (the examples' probe inputs): the number of parts the load is split into.
pub const PROBE_LOAD_PARTS: &str = "PG_PROBE_LOAD_PARTS";
/// A probe input, not the app's (the examples' probe inputs): set, the pass pairs what the catalog owes.
pub const PROBE_KEEP_OWED: &str = "PG_PROBE_KEEP_OWED";
/// A probe input, not the app's (the examples' probe inputs): set, every write reads its pictures' analyses.
pub const PROBE_GUARD: &str = "PG_PROBE_GUARD";
/// A probe input, not the app's (the examples' probe inputs): the join's few-entries threshold.
pub const PROBE_FEW: &str = "PG_PROBE_FEW";
/// A probe input, not the app's (the examples' probe inputs): the pairing pass's page cache in KiB.
pub const PROBE_CACHE_KIB: &str = "PG_PROBE_CACHE_KIB";
/// A probe input, not the app's (the examples' probe inputs): the query buckets of a scan task.
pub const PROBE_BLOCK: &str = "PG_PROBE_BLOCK";
/// A probe input, not the app's (the examples' probe inputs): set to `0`, the statement-journal probe skips its memory
/// statistics.
pub const PROBE_MEMSTAT: &str = "PG_PROBE_MEMSTAT";

/// A dump input of the join test (the join test's dump inputs): the number of threads the join runs on.
pub const JOIN_THREADS: &str = "PG_JOIN_THREADS";
/// A dump input of the join test (the join test's dump inputs): the file the pairs are written to.
pub const JOIN_OUT: &str = "PG_JOIN_OUT";
/// A dump input of the join test (the join test's dump inputs): the catalog dump the join reads its hashes from.
pub const JOIN_DUMP: &str = "PG_JOIN_DUMP";

/// A benchmark's output label: the name printed on the benchmark's result line.
pub const LABEL: &str = "PG_LABEL";

/// An input a test or a rehearsal reads: a real catalog the ignored catalog tests open read-only.
pub const TEST_CATALOG: &str = "PG_TEST_CATALOG";
/// An input a test or a rehearsal reads: the real catalog a rehearsal copies and runs on; the original is never opened
/// for writing.
pub const REHEARSAL_CATALOG: &str = "PG_REHEARSAL_CATALOG";
/// An input a test or a rehearsal reads: the folder of real JPEGs the ignored comparison decodes with both decoders.
pub const JPEG_COMPARE_DIR: &str = "PG_JPEG_COMPARE_DIR";
