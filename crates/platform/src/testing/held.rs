//! A file that another program holds, so that this process can't read it or can't write it: the state a test needs to see
//! a read or a write fail. Windows holds it by opening it without sharing, which fails every other open of it while the
//! hold lasts. Unix has no sharing modes, so the file's permissions stop the access instead, and they are put back when
//! the hold drops. Where the permissions don't stop this process (Unix as root), there is no hold: [`hold`] says so, and
//! the test skips.

use std::path::Path;

/// What the hold stops this process from doing to the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// The hold on a file, until it drops.
#[cfg(windows)]
#[derive(Debug)]
pub struct Held {
    _file: std::fs::File,
}

/// The hold on a file, until it drops: the file's permissions are put back then.
#[cfg(unix)]
#[derive(Debug)]
pub struct Held {
    path: std::path::PathBuf,
    before: std::fs::Permissions,
}

/// The hold on `path` against `access`, or `None` where this process isn't stopped by one (Unix as root: the
/// permissions don't bind it). The file must exist.
#[cfg(windows)]
pub fn hold(path: &Path, _access: Access) -> std::io::Result<Option<Held>> {
    use std::os::windows::fs::OpenOptionsExt;
    // No sharing at all: another open of the file, for reading or for writing, fails while this handle is open.
    let file = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(path)?;
    Ok(Some(Held { _file: file }))
}

/// The hold on `path` against `access`, or `None` where this process isn't stopped by one (Unix as root: the
/// permissions don't bind it). The file must exist.
#[cfg(unix)]
pub fn hold(path: &Path, access: Access) -> std::io::Result<Option<Held>> {
    use std::os::unix::fs::PermissionsExt;
    let before = std::fs::metadata(path)?.permissions();
    let mode = match access {
        Access::Read => 0o000,
        Access::Write => 0o444,
    };
    let held = Held { path: path.to_path_buf(), before };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    let stopped = match access {
        Access::Read => std::fs::File::open(path).is_err(),
        Access::Write => std::fs::OpenOptions::new().write(true).open(path).is_err(),
    };
    // Not stopped: `held` drops here, and puts the permissions back.
    Ok(stopped.then_some(held))
}

#[cfg(unix)]
impl Drop for Held {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.path, self.before.clone());
    }
}
