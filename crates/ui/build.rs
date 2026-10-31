use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    let mut builder = CxxQtBuilder::new_qml_module(
        QmlModule::new("pg.gallery")
            .qml_files(["qml/Main.qml", "qml/Viewer.qml", "qml/WheelScroller.qml"])
            .qml_file(QmlFile::from("qml/Ratings.qml").singleton(true)),
    )
    .qt_module("Quick")
    .files([
        "src/app_icon.rs",
        "src/folders.rs",
        "src/gallery.rs",
        "src/library.rs",
        "src/providers.rs",
        "src/settings.rs",
    ])
    .cpp_file("cpp/app_icon.cpp")
    .cpp_file("cpp/providers.cpp")
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
