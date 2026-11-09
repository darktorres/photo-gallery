//! The per-user folder the app keeps its data in when nothing says where: `%LOCALAPPDATA%\PhotoGallery` on Windows, and
//! `$XDG_DATA_HOME/photo-gallery` (or `~/.local/share/photo-gallery`) elsewhere. The environment is read through a lookup,
//! so the tests give it one; the callers pass `std::env::var_os`.

use std::ffi::OsString;
use std::path::PathBuf;

/// The default data folder under the environment `var` gives, for the OS this is built for. An error, never the temporary
/// folder, when no variable gives an absolute folder: a catalog in a temporary folder is deleted by the system.
pub fn default_home(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    home_for(cfg!(windows), var)
}

/// [`default_home`] for the named OS, so both are tested on every host.
fn home_for(windows: bool, var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    let absolute =
        |name: &str| var(name).filter(|value| is_absolute_for(windows, &value.to_string_lossy())).map(PathBuf::from);
    if windows {
        return absolute("LOCALAPPDATA").map(|local| local.join("PhotoGallery")).ok_or_else(|| {
            "LOCALAPPDATA is not set to a folder, so there is no default folder for the library: set PG_HOME"
                .to_string()
        });
    }
    absolute("XDG_DATA_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local").join("share")))
        .map(|data| data.join("photo-gallery"))
        .ok_or_else(|| {
            "neither XDG_DATA_HOME nor HOME is an absolute folder, so there is no default folder for the library: set PG_HOME"
                .to_string()
        })
}

/// Whether `path` is absolute in the named OS's form: a drive (`C:\`) or a UNC share on Windows, a leading `/` elsewhere.
/// Checked by its text, so the other OS's paths are judged the way that OS judges them.
fn is_absolute_for(windows: bool, path: &str) -> bool {
    if windows {
        let drive =
            matches!(path.as_bytes(), [d, b':', s, ..] if d.is_ascii_alphabetic() && (*s == b'\\' || *s == b'/'));
        drive || path.starts_with("\\\\")
    } else {
        path.starts_with('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: BTreeMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn windows_uses_local_app_data() {
        let home = home_for(true, env(&[("LOCALAPPDATA", r"C:\Users\a\AppData\Local")])).unwrap();
        assert_eq!(home, PathBuf::from(r"C:\Users\a\AppData\Local").join("PhotoGallery"));
    }

    #[test]
    fn linux_uses_xdg_data_home_when_it_is_absolute() {
        let home = home_for(false, env(&[("XDG_DATA_HOME", "/srv/data"), ("HOME", "/home/a")])).unwrap();
        assert_eq!(home, PathBuf::from("/srv/data/photo-gallery"));
    }

    #[test]
    fn linux_ignores_a_relative_xdg_data_home() {
        let home = home_for(false, env(&[("XDG_DATA_HOME", "data"), ("HOME", "/home/a")])).unwrap();
        assert_eq!(home, PathBuf::from("/home/a/.local/share/photo-gallery"));
    }

    #[test]
    fn linux_falls_back_to_home() {
        let home = home_for(false, env(&[("HOME", "/home/a")])).unwrap();
        assert_eq!(home, PathBuf::from("/home/a/.local/share/photo-gallery"));
    }

    #[test]
    fn nothing_found_is_an_error_not_the_temporary_folder() {
        assert!(home_for(false, env(&[("TMPDIR", "/tmp")])).is_err());
        assert!(home_for(true, env(&[])).is_err());
    }
}
