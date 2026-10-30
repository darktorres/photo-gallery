use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("pg.gallery").qml_files(["qml/Main.qml", "qml/Viewer.qml", "qml/WheelScroller.qml"]))
        .qt_module("Quick")
        .files(["src/folders.rs", "src/gallery.rs", "src/library.rs", "src/providers.rs"])
        .cpp_file("cpp/providers.cpp")
        .build();
}
