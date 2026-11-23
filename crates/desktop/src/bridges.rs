//! The cxx-qt and cxx bridges: the declarations the Qt side and the feature modules share. Each `<name>_bridge` is the
//! bridge of `crate::<name>`, and each `<name>_state` is the state that bridge's QObject holds (the struct its generated
//! code names in a public interface, so it is `pub` and lives here with it). `app_icon`, `clipboard` and `live_reload`
//! are the window's own C++ calls. They sit in one directory because cxx-qt-build takes one directory per QML module
//! (build.rs lists the files), so this module and its expectations in `lib.rs` cover them all.

pub(crate) mod animation_bridge;
pub(crate) mod animation_state;
pub(crate) mod app_icon;
pub(crate) mod clipboard;
pub(crate) mod command_line_bridge;
pub(crate) mod command_line_state;
pub(crate) mod deadline_bridge;
pub(crate) mod facet_choices_bridge;
pub(crate) mod facet_choices_state;
pub(crate) mod failed_files_bridge;
pub(crate) mod failed_files_state;
pub(crate) mod filters_bridge;
pub(crate) mod filters_state;
pub(crate) mod folders_bridge;
pub(crate) mod folders_state;
pub(crate) mod frame_sink_bridge;
pub(crate) mod gallery_bridge;
pub(crate) mod gallery_state;
pub(crate) mod gpu_device_bridge;
pub(crate) mod gui_app_bridge;
pub(crate) mod keymap_bridge;
pub(crate) mod keymap_state;
pub(crate) mod lane_model_bridge;
pub(crate) mod lane_model_state;
pub(crate) mod library_bridge;
pub(crate) mod library_state;
#[cfg(debug_assertions)]
pub(crate) mod live_reload;
pub(crate) mod log_model_bridge;
pub(crate) mod log_model_state;
pub(crate) mod notices_bridge;
pub(crate) mod notices_state;
pub(crate) mod picker_bridge;
pub(crate) mod picker_state;
pub(crate) mod providers_bridge;
pub(crate) mod screenshot_bridge;
pub(crate) mod settings_bridge;
pub(crate) mod settings_state;
pub(crate) mod shell_bridge;
pub(crate) mod shell_state;
pub(crate) mod trace_bridge;
pub(crate) mod trace_state;
pub(crate) mod video_player_bridge;
pub(crate) mod video_source_bridge;
pub(crate) mod video_source_state;
pub(crate) mod viewport_bridge;
pub(crate) mod viewport_state;
pub(crate) mod words_bridge;
pub(crate) mod words_state;
pub(crate) mod yandex_bridge;
pub(crate) mod yandex_state;
