use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    // The QML is compiled to C++ (qmlcachegen) against this module's types: qt-build-utils is vendored for that
    // (vendor/README.md).
    let mut builder = CxxQtBuilder::new_qml_module(
        QmlModule::new("pg.gallery")
            // The models' base class (QAbstractListModel) is declared there: without it in the qmldir, the QML
            // compiler can't resolve GalleryModel and FolderModel, and compiles nothing that uses them.
            .depend("QtQml.Models")
            .qml_files([
                "qml/Main.qml",
                "qml/FilterBar.qml",
                "qml/Viewer.qml",
                "qml/WheelListView.qml",
                "qml/YandexPanel.qml",
            ])
            .qml_file(QmlFile::from("qml/Ratings.qml").singleton(true)),
    )
    .qt_module("Quick")
    .qt_module("Multimedia")
    .files([
        "src/animation.rs",
        "src/app_icon.rs",
        "src/command_line.rs",
        "src/filters.rs",
        "src/folders.rs",
        "src/gallery.rs",
        "src/library.rs",
        "src/providers.rs",
        "src/screenshot.rs",
        "src/settings.rs",
        "src/trace.rs",
        "src/video_decoding.rs",
        "src/view.rs",
        "src/yandex.rs",
    ])
    .cpp_file("cpp/app_icon.cpp")
    .cpp_file("cpp/frame_sink.cpp")
    .cpp_file("cpp/providers.cpp")
    .cpp_file("cpp/screenshot.cpp")
    .cpp_file("cpp/video_decoding.cpp")
    // The window icon, set in app_icon.cpp (drawn by tools/make_icon.py).
    .qrc_resources(["resources/app-icon.png"]);
    // Debug builds read the QML from the source folder and reload it when it changes (src/live_reload.rs).
    if std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
        builder = builder.file("src/live_reload.rs").cpp_file("cpp/live_reload.cpp");
    }
    builder.build();

    // The icon of gallery.exe, which Explorer and the taskbar show (made by tools/make_icon.py). The resource compiler
    // is part of the Windows SDK. The target's OS, not this script's host, decides: the script's cfg is the host's.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        println!("cargo:rerun-if-changed=resources/app.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon("resources/app.ico");
        res.compile().expect("embedding resources/app.ico in gallery.exe");
    }
}
