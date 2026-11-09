//! The console of a program that the system starts as a GUI program: the console it was typed in, attached to its standard
//! streams.

/// Make stdout and stderr reach the console the command was typed in. A GUI-subsystem program starts without them unless
/// they were redirected (a pipe or a file, which then stay as they are).
#[cfg(windows)]
pub fn attach_to_parent() {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    let valid = |h: HANDLE| !h.is_null() && h != INVALID_HANDLE_VALUE;
    // SAFETY: GetStdHandle and AttachConsole take no pointers.
    let missing: Vec<STD_HANDLE> =
        [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().filter(|&s| !valid(unsafe { GetStdHandle(s) })).collect();
    // SAFETY: as above.
    if missing.is_empty() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: `name` is NUL-terminated and outlives the call; the null pointers are optional arguments.
    let out = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if valid(out) {
        for s in missing {
            // SAFETY: `out` is a valid console handle, kept open for the rest of the process.
            unsafe { SetStdHandle(s, out) };
        }
    }
}

/// Nothing to attach elsewhere: a program's standard streams are its caller's from the start.
#[cfg(not(windows))]
pub fn attach_to_parent() {}
