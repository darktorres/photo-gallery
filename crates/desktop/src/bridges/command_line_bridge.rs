//! The cxx-qt bridge of [`crate::command_line`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here. The `CommandLine` object's properties are the test hooks that
//! TestHooks.qml still reads, each typed as its option is (the "not given" values are documented on the reader). The
//! other hooks are run in Rust (`test_hooks`).

use crate::bridges::command_line_state::CommandLineRust;

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The mascot to show (`--mascot`): -1 for a random one.
        #[qproperty(i32, mascot, READ = read_mascot, FINAL)]
        /// The search result to show at full size (`--yandex-preview`): -1 when none.
        #[qproperty(i32, yandex_preview, cxx_name = "yandexPreview", READ = read_yandex_preview, FINAL)]
        /// The window is being photographed (`--screenshot`): its carets are not drawn, since they blink.
        #[qproperty(bool, screenshot, READ = read_screenshot, FINAL)]
        type CommandLine = super::CommandLineRust;
    }

    unsafe extern "RustQt" {
        fn read_mascot(self: &CommandLine) -> i32;
        fn read_yandex_preview(self: &CommandLine) -> i32;
        fn read_screenshot(self: &CommandLine) -> bool;
    }
}
