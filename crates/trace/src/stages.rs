//! The stage functions: each is exported by name (`no_mangle`), so a stage's symbol is findable in a dump. Their lint is
//! allowed on the module, because a lint on a macro call is reported at the call.

#![expect(unsafe_code, reason = "the stage functions are exported by name (`no_mangle`) for a dump to find")]

use crate::sink::reached;

/// Each body names its stage, so no two are identical and the linker can't fold them into one function.
macro_rules! stages {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {
        $(
            $(#[$doc])*
            #[unsafe(no_mangle)]
            #[inline(never)]
            pub extern "C" fn $name() {
                reached(stringify!($name));
            }
        )*
    };
}

stages! {
    /// `main` runs (the DLLs are loaded).
    pg_stage_main,
    /// The catalog and the thumbnail cache are open.
    pg_stage_init_done,
    /// The QML engine starts loading the window.
    pg_stage_qml_load,
    /// It has (the window is up, not yet painted).
    pg_stage_qml_loaded,
    /// The first frame is on screen.
    pg_stage_first_frame,
    /// A folder tree is shown.
    pg_stage_tree_shown,
    /// A grid list is shown.
    pg_stage_list_shown,
    /// The start is done.
    pg_stage_ready,
    /// The library's background job finished a round (on its thread; the first is the start's import check and
    /// thumbnail pass).
    pg_stage_round_done,
}
