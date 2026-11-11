//! Moving a database file aside and deleting it, each with its sidecars (`-wal`, `-shm`, `-journal`). The renames never
//! replace a file (`pg_platform::file::rename_no_replace`).

use std::io;
use std::path::{Path, PathBuf};

use pg_platform::file::{beside, rename_no_replace};

/// A database file's parts, the sidecars first: a `-wal` left beside a main file that moved would be replayed into
/// whatever takes the name next.
const PARTS: [&str; 4] = ["-wal", "-shm", "-journal", ""];

/// Move the database at `path`, with its `-wal`, `-shm` and `-journal` files, aside to the first free name beside it:
/// `<name><mark>`, then `<name><mark>.1`, and so on (a catalog that is replaced, `.replaced`; a thumbnail cache that is
/// damaged, `.bad`). Returns the main file's new path. Nothing is deleted and nothing is overwritten: the user removes
/// the moved files when they are no longer wanted. The sidecars move first, so the main file is never left at `path`
/// without the changes its sidecars held. The parts share one name, so they are found free together; no connection to
/// the database may be open, and the caller holds its [`crate::Lock`] (and its exclusive [`crate::Live`]), so no other process of
/// ours takes a name between the check and the move. Each move never replaces a file all the same
/// (`pg_platform::file::rename_no_replace`): one another program made in between fails the move, and is kept.
pub fn move_aside(path: &Path, mark: &str) -> io::Result<PathBuf> {
    let mut n = 0u32;
    let base = loop {
        let base = if n == 0 { mark.to_string() } else { format!("{mark}.{n}") };
        if PARTS.iter().all(|part| !beside(path, &format!("{base}{part}")).exists()) {
            break base;
        }
        n += 1;
    };
    for part in PARTS {
        let from = beside(path, part);
        if from.exists() {
            let to = beside(path, &format!("{base}{part}"));
            rename_no_replace(&from, &to)
                .map_err(|e| io::Error::new(e.kind(), format!("can't move {} aside: {e}", from.display())))?;
        }
    }
    Ok(beside(path, &base))
}

/// Delete the database at `path` with its sidecars, the sidecars first: a `-wal` left beside a new file of the name would
/// be replayed into it. A part that isn't there is no error. No connection to the database may be open, and the caller
/// holds its exclusive [`crate::Live`], so no other process has it open.
pub fn remove_database(path: &Path) -> io::Result<()> {
    for part in PARTS {
        let file = beside(path, part);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io::Error::new(e.kind(), format!("can't delete {}: {e}", file.display()))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The main file and its sidecars move together to the first free name, and nothing already there is overwritten.
    #[test]
    fn moving_aside_takes_the_sidecars_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("pg-dbfile-aside-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        std::fs::write(&path, b"first").unwrap();
        std::fs::write(dir.join("db.sqlite-wal"), b"first wal").unwrap();
        std::fs::write(dir.join("db.sqlite.bad-shm"), b"in the way").unwrap();
        // `.bad` is taken (one of its parts is there), so `.bad.1` is the first free name.
        let kept = move_aside(&path, ".bad").unwrap();
        assert_eq!(kept, dir.join("db.sqlite.bad.1"));
        assert_eq!(std::fs::read(&kept).unwrap(), b"first");
        assert_eq!(std::fs::read(dir.join("db.sqlite.bad.1-wal")).unwrap(), b"first wal");
        assert_eq!(std::fs::read(dir.join("db.sqlite.bad-shm")).unwrap(), b"in the way");
        assert!(!path.exists() && !dir.join("db.sqlite-wal").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removing_takes_the_sidecars_and_a_missing_part_is_no_error() {
        let dir = std::env::temp_dir().join(format!("pg-dbfile-remove-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        std::fs::write(&path, b"main").unwrap();
        std::fs::write(dir.join("db.sqlite-wal"), b"wal").unwrap();
        remove_database(&path).unwrap();
        assert!(!path.exists() && !dir.join("db.sqlite-wal").exists());
        remove_database(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
