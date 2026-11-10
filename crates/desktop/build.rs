use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

/// A failure is an error (cargo prints it and the build stops).
fn main() -> Result<(), String> {
    // The mascots: resources/mascots/01.png to 40.png, in name order. The path line is for local builds: with the content key
    // (CI sets PG_DESKTOP_CONTENT_KEY, see below), the build reruns on the key, and the folder's mtime does not count.
    if !has_content_key() {
        println!("cargo:rerun-if-changed=resources/mascots");
    }
    let mut resources = vec![String::from("resources/app-icon.png")];
    let entries = std::fs::read_dir("resources/mascots").map_err(|e| format!("reading resources/mascots: {e}"))?;
    let mut mascots = entries
        .map(|entry| {
            entry
                .map(|entry| format!("resources/mascots/{}", entry.file_name().to_string_lossy()))
                .map_err(|e| format!("reading resources/mascots: {e}"))
        })
        .collect::<Result<Vec<String>, String>>()?;
    mascots.sort();
    resources.extend(mascots);

    // The QML is compiled to C++ (qmlcachegen) against this module's types: qt-build-utils is vendored for that
    // (vendor/README.md).
    let mut builder = CxxQtBuilder::new_qml_module(
        QmlModule::new("pg.gallery")
            // The models' base class (QAbstractListModel) is declared there: without it in the qmldir, the QML
            // compiler can't resolve GalleryModel and FolderModel, and compiles nothing that uses them.
            .depend("QtQml.Models")
            .qml_files([
                "qml/Main.qml",
                "qml/BorderOverlay.qml",
                "qml/FilterBar.qml",
                "qml/Viewer.qml",
                "qml/WheelListView.qml",
                "qml/YandexPanel.qml",
                "qml/YandexPreview.qml",
                "qml/ResultTile.qml",
                "qml/TopBar.qml",
                "qml/ViewsMenu.qml",
                "qml/StatusBar.qml",
                "qml/FolderTree.qml",
                "qml/FolderPickerDialog.qml",
                "qml/GalleryGrid.qml",
                "qml/GridRow.qml",
                "qml/GridCell.qml",
                "qml/LibraryPanel.qml",
                "qml/SaveViewDialog.qml",
                "qml/ModalOverlay.qml",
                "qml/TestHooks.qml",
                "qml/ImageLayers.qml",
                "qml/MediaView.qml",
                "qml/VideoTransport.qml",
                "qml/Loupe.qml",
                "qml/RatingBar.qml",
                "qml/RatingPopup.qml",
                "qml/Hud.qml",
                "qml/Mascot.qml",
                "qml/GlobalShortcuts.qml",
                "qml/LogPanel.qml",
                "qml/PanelKind.qml",
            ])
            .qml_file(QmlFile::from("qml/Ratings.qml").singleton(true))
            .qml_file(QmlFile::from("qml/Theme.qml").singleton(true)),
    )
    // Before the files and resources below: the builder emits their path lines when they are added (vendor/README.md).
    // CI sets PG_DESKTOP_CONTENT_KEY to a hash of this crate, so the build reruns when the crate changes and not when a
    // checkout makes its files newer. Local builds leave it unset and use the path lines.
    .content_key_env("PG_DESKTOP_CONTENT_KEY")
    .qt_module("Quick")
    .qt_module("Multimedia")
    .files([
        "src/animation_bridge.rs",
        "src/frame_sink_bridge.rs",
        "src/app_icon.rs",
        "src/command_line_bridge.rs",
        "src/deadline_bridge.rs",
        "src/facet_choices_bridge.rs",
        "src/filters_bridge.rs",
        "src/folders_bridge.rs",
        "src/gallery_bridge.rs",
        "src/gpu_device_bridge.rs",
        "src/gui_app_bridge.rs",
        "src/picker_bridge.rs",
        "src/library_bridge.rs",
        "src/log_model_bridge.rs",
        "src/providers_bridge.rs",
        "src/screenshot_bridge.rs",
        "src/settings_bridge.rs",
        "src/shell_bridge.rs",
        "src/trace_bridge.rs",
        "src/keymap_bridge.rs",
        "src/video_source_bridge.rs",
        "src/video_player_bridge.rs",
        "src/viewport_bridge.rs",
        "src/words_bridge.rs",
        "src/yandex_bridge.rs",
    ])
    .cpp_file("cpp/app_icon.cpp")
    .cpp_file("cpp/deadline.cpp")
    .cpp_file("cpp/picker.cpp")
    .cpp_file("cpp/frame_sink.cpp")
    .cpp_file("cpp/gpu_device.cpp")
    .cpp_file("cpp/gui_app.cpp")
    .cpp_file("cpp/providers.cpp")
    .cpp_file("cpp/screenshot.cpp")
    .cpp_file("cpp/video_source.cpp")
    // The window icon, set in app_icon.cpp (drawn by tools/make_icon.py), and the mascots Mascot.qml picks from.
    .qrc_resources(resources);
    // Debug builds read the QML from the source folder and reload it when it changes (src/live_reload.rs).
    if std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
        builder = builder.file("src/live_reload.rs").cpp_file("cpp/live_reload.cpp");
    }
    builder.build();
    Ok(())
}

/// Whether the content key is set (non-empty), as CI sets it: the builder then emits no path lines for the sources.
fn has_content_key() -> bool {
    std::env::var("PG_DESKTOP_CONTENT_KEY").is_ok_and(|key| !key.is_empty())
}
