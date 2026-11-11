//! The program's arguments as the system holds them: one argument's bytes for the list Qt is given ([`arg_bytes`]),
//! and a `--name=value` argument split at its first `=`, exact ([`split_at_first_equals`]).

use std::ffi::{OsStr, OsString};

/// One argument's bytes, as the program's own argument list holds them: exactly on Unix, a raw byte included. On Windows,
/// its text, converted lossily, as cxx-qt-lib passes arguments to Qt (Qt reads its own arguments from the command line there,
/// exactly; the app reads its arguments from its command line, so the list only has to be well formed).
#[cfg(unix)]
pub fn arg_bytes(arg: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    arg.as_bytes().to_vec()
}

/// One argument's bytes: its text (lossy), as cxx-qt-lib passes them (see the Unix one).
#[cfg(windows)]
pub fn arg_bytes(arg: &OsStr) -> Vec<u8> {
    arg.to_string_lossy().as_bytes().to_vec()
}

/// `arg`'s parts before and after its first `=`, as the system holds them (none without a `=`).
#[cfg(unix)]
pub fn split_at_first_equals(arg: &OsStr) -> Option<(OsString, OsString)> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let bytes = arg.as_bytes();
    let at = bytes.iter().position(|&b| b == b'=')?;
    let (before, after) = (bytes.get(..at)?, bytes.get(at + 1..)?);
    Some((OsString::from_vec(before.to_vec()), OsString::from_vec(after.to_vec())))
}

/// `arg`'s parts before and after its first `=`, as the system holds them (none without a `=`).
#[cfg(windows)]
pub fn split_at_first_equals(arg: &OsStr) -> Option<(OsString, OsString)> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> = arg.encode_wide().collect();
    let at = units.iter().position(|&u| u == u16::from(b'='))?;
    let (before, after) = (units.get(..at)?, units.get(at + 1..)?);
    Some((OsString::from_wide(before), OsString::from_wide(after)))
}
