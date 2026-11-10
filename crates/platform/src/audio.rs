//! Qt's PipeWire audio output, and whether PipeWire can start here. Qt 6.12 uses PipeWire for audio when its library is
//! installed, and then dereferences a NULL PipeWire context if PipeWire cannot make one. A PipeWire install without its
//! client configuration (`client.conf`, which the `pipewire` package ships, not the libraries) is one such case: its
//! `pw_context_new` returns NULL. The app asks Qt for PulseAudio in that case (crates/desktop/src/lib.rs).

#[cfg(target_os = "linux")]
use std::ffi::{c_char, c_int, c_void};
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::ptr;

#[cfg(target_os = "linux")]
use libloading::Library;

/// Whether PipeWire's library is installed and cannot make a context, the way Qt's startup makes one: a thread loop
/// started, then a context on its loop (no daemon is needed). False where the library is absent (Qt does not use PipeWire
/// then), where PipeWire works, and off Linux.
#[must_use]
pub fn pipewire_context_fails() -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: loading PipeWire's library runs its initializers, as Qt's own load of it does.
        let Ok(lib) = (unsafe { crate::dll::load(Path::new("libpipewire-0.3.so.0")) }) else {
            return false;
        };
        symbols_fail(&lib).unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
type Init = unsafe extern "C" fn(*mut c_int, *mut c_void);
#[cfg(target_os = "linux")]
type LoopNew = unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void;
#[cfg(target_os = "linux")]
type LoopStart = unsafe extern "C" fn(*mut c_void) -> c_int;
#[cfg(target_os = "linux")]
type LoopCall = unsafe extern "C" fn(*mut c_void);
#[cfg(target_os = "linux")]
type LoopGetLoop = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
#[cfg(target_os = "linux")]
type ContextNew = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void;
#[cfg(target_os = "linux")]
type ContextDestroy = unsafe extern "C" fn(*mut c_void);

/// The startup itself, in the order Qt makes its calls. `None` when a function is missing (the check is then not made).
#[cfg(target_os = "linux")]
fn symbols_fail(lib: &Library) -> Option<bool> {
    let init: Init = symbol(lib, b"pw_init\0")?;
    let loop_new: LoopNew = symbol(lib, b"pw_thread_loop_new\0")?;
    let loop_start: LoopStart = symbol(lib, b"pw_thread_loop_start\0")?;
    let loop_stop: LoopCall = symbol(lib, b"pw_thread_loop_stop\0")?;
    let loop_lock: LoopCall = symbol(lib, b"pw_thread_loop_lock\0")?;
    let loop_unlock: LoopCall = symbol(lib, b"pw_thread_loop_unlock\0")?;
    let loop_destroy: LoopCall = symbol(lib, b"pw_thread_loop_destroy\0")?;
    let loop_get_loop: LoopGetLoop = symbol(lib, b"pw_thread_loop_get_loop\0")?;
    let context_new: ContextNew = symbol(lib, b"pw_context_new\0")?;
    let context_destroy: ContextDestroy = symbol(lib, b"pw_context_destroy\0")?;

    // SAFETY: each call is PipeWire's own, with the arguments its header declares: null options, the loop started before
    // the context is made on it, and the context destroyed under the lock before the loop is stopped and destroyed.
    unsafe {
        init(ptr::null_mut(), ptr::null_mut());
        let thread_loop = loop_new(c"PwEventLoop".as_ptr(), ptr::null());
        if thread_loop.is_null() {
            return Some(true);
        }
        let started = loop_start(thread_loop) == 0;
        let mut context = ptr::null_mut();
        if started {
            loop_lock(thread_loop);
            context = context_new(loop_get_loop(thread_loop), ptr::null_mut(), 0);
            loop_unlock(thread_loop);
        }
        let fails = context.is_null();
        if !context.is_null() {
            loop_lock(thread_loop);
            context_destroy(context);
            loop_unlock(thread_loop);
        }
        if started {
            loop_stop(thread_loop);
        }
        loop_destroy(thread_loop);
        Some(fails)
    }
}

/// The function `name` of `lib`, as the type `T` (a C function pointer). `None` when the library does not have it.
#[cfg(target_os = "linux")]
fn symbol<T: Copy>(lib: &Library, name: &[u8]) -> Option<T> {
    // SAFETY: the caller names a function with the C signature `T` declares, as PipeWire's headers do.
    unsafe { lib.get::<T>(name).ok().map(|symbol| *symbol) }
}

#[cfg(test)]
mod tests {
    use super::pipewire_context_fails;

    // Only meaningful where PipeWire's client configuration is missing: run it with PIPEWIRE_CONFIG_DIR set to an empty
    // folder (`cargo test -p pg-platform audio -- --ignored`), where it reports the failure Qt crashes on.
    #[test]
    #[ignore = "needs PipeWire's client configuration missing: run with PIPEWIRE_CONFIG_DIR set to an empty folder"]
    fn a_missing_client_configuration_is_reported() {
        assert!(pipewire_context_fails());
    }

    #[test]
    fn the_check_does_not_panic_here() {
        // The answer depends on the machine (a PipeWire install with its configuration gives false); the check itself must
        // return either way.
        let _ = pipewire_context_fails();
    }
}
