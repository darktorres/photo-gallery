use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    let mut builder = CxxQtBuilder::new_qml_module(
        QmlModule::new("pg.gallery")
            .qml_files(["qml/Main.qml", "qml/Viewer.qml", "qml/WheelScroller.qml"])
            .qml_file(QmlFile::from("qml/Ratings.qml").singleton(true)),
    )
    .qt_module("Quick")
    .files(["src/folders.rs", "src/gallery.rs", "src/library.rs", "src/providers.rs", "src/settings.rs"])
    .cpp_file("cpp/providers.cpp");
    // Debug builds read the QML from the source folder and reload it when it changes (src/live_reload.rs).
    if std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
        builder = builder.file("src/live_reload.rs").cpp_file("cpp/live_reload.cpp");
    }
    builder.build();
}
