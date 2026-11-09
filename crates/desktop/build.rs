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

    // The `pg_static_qt` cfg (set below for the static build) is declared for every build, so the shared build is
    // checked the same way.
    println!("cargo::rustc-check-cfg=cfg(pg_static_qt)");

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
        "src/animation.rs",
        "src/app_icon.rs",
        "src/command_line.rs",
        "src/deadline.rs",
        "src/facet_choices.rs",
        "src/filters.rs",
        "src/folders.rs",
        "src/gallery.rs",
        "src/gpu_device.rs",
        "src/gui_app.rs",
        "src/picker.rs",
        "src/library.rs",
        "src/providers.rs",
        "src/screenshot.rs",
        "src/settings.rs",
        "src/shell.rs",
        "src/trace.rs",
        "src/keymap.rs",
        "src/video_source.rs",
        "src/viewport.rs",
        "src/words.rs",
        "src/yandex.rs",
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
    // A static Qt (PG_QT_STATIC set; docs/STATIC_BUILD.md): the modules the QML uses, the plugins' registration
    // (cpp/static_qt.cpp), and the libraries of the static link. Only that build sets the variable: the shared build is
    // unchanged.
    if std::env::var_os("PG_QT_STATIC").is_some() {
        builder = builder
            .qt_module("QuickControls2")
            .qt_module("QuickLayouts")
            .qt_module("Svg")
            .cpp_file("cpp/static_qt.cpp");
        static_qt_links()?;
    }
    builder.build();
    Ok(())
}

/// The static Qt's libraries beyond the modules cxx-qt links: the plugins and QML plugins (paths under QT_DIR, the static
/// Qt's prefix), the Windows libraries Qt's static libraries need, and the static FFmpeg that Qt's FFmpeg plugin uses
/// (PG_FFMPEG_STATIC, its lib folder). The lists are the static link line of the check program (docs/STATIC_BUILD.md).
///
/// The libraries go out as `rustc-link-search` and `rustc-link-lib`, which reach the binary that depends on this crate.
/// `rustc-link-arg` would not: it is for the crate's own link, so the plugins would be lost (the app would start without
/// its platform plugin). The `pg_static_qt` cfg tells gui_app.rs to keep cpp/static_qt.cpp's registrations linked.
fn static_qt_links() -> Result<(), String> {
    let qt = std::path::PathBuf::from(
        std::env::var_os("QT_DIR").ok_or("PG_QT_STATIC needs QT_DIR, the static Qt's prefix")?,
    );
    let ffmpeg = std::path::PathBuf::from(
        std::env::var_os("PG_FFMPEG_STATIC").ok_or("PG_QT_STATIC needs PG_FFMPEG_STATIC, the static FFmpeg's lib folder")?,
    );
    println!("cargo::rustc-cfg=pg_static_qt");
    for lib in STATIC_QT_LIBS {
        link_static(&qt.join(lib))?;
    }
    for lib in STATIC_FFMPEG_LIBS {
        link_static(&ffmpeg.join(format!("{lib}.lib")))?;
    }
    for lib in STATIC_QT_SYSTEM_LIBS {
        println!("cargo::rustc-link-lib={lib}");
    }
    Ok(())
}

/// Links one static library by its file: its folder is searched, and its name (without `.lib`) is the library.
fn link_static(path: &std::path::Path) -> Result<(), String> {
    let dir = path.parent().ok_or_else(|| format!("{}: no folder", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: no library name", path.display()))?;
    println!("cargo::rustc-link-search=native={}", dir.display());
    println!("cargo::rustc-link-lib=static={name}");
    Ok(())
}

/// Qt's static plugin and QML plugin libraries the app imports (cpp/static_qt.cpp), relative to QT_DIR.
const STATIC_QT_LIBS: &[&str] = &[
    "lib/Qt6FFmpegMediaPluginImpl.lib",
    // The modules the QML plugins below are built on (their .prl files name them, and qt-build-utils reads only the
    // modules' .prl files, not these).
    "lib/Qt6QuickControls2Impl.lib",
    "lib/Qt6QuickControls2Basic.lib",
    "lib/Qt6QuickControls2BasicStyleImpl.lib",
    "lib/Qt6QuickControls2Fusion.lib",
    "lib/Qt6QuickControls2FusionStyleImpl.lib",
    "lib/Qt6MultimediaQuick.lib",
    "plugins/platforms/qwindows.lib",
    "plugins/platforms/qoffscreen.lib",
    "plugins/imageformats/qgif.lib",
    "plugins/imageformats/qicns.lib",
    "plugins/imageformats/qico.lib",
    "plugins/imageformats/qjpeg.lib",
    "plugins/imageformats/qsvg.lib",
    "plugins/imageformats/qtga.lib",
    "plugins/imageformats/qtiff.lib",
    "plugins/imageformats/qwbmp.lib",
    "plugins/imageformats/qwebp.lib",
    "plugins/tls/qschannelbackend.lib",
    "plugins/tls/qcertonlybackend.lib",
    "plugins/multimedia/ffmpegmediaplugin.lib",
    "plugins/multimedia/windowsmediaplugin.lib",
    "plugins/iconengines/qsvgicon.lib",
    "qml/QtQml/qmlplugin.lib",
    "qml/QtQml/Models/modelsplugin.lib",
    "qml/QtQml/WorkerScript/workerscriptplugin.lib",
    "qml/QtQuick/qtquick2plugin.lib",
    "qml/QtQuick/Window/quickwindowplugin.lib",
    "qml/QtQuick/Layouts/qquicklayoutsplugin.lib",
    "qml/QtQuick/Templates/qtquicktemplates2plugin.lib",
    "qml/QtQuick/Controls/qtquickcontrols2plugin.lib",
    "qml/QtQuick/Controls/impl/qtquickcontrols2implplugin.lib",
    "qml/QtQuick/Controls/Basic/qtquickcontrols2basicstyleplugin.lib",
    "qml/QtQuick/Controls/Basic/impl/qtquickcontrols2basicstyleimplplugin.lib",
    "qml/QtQuick/Controls/Fusion/qtquickcontrols2fusionstyleplugin.lib",
    "qml/QtQuick/Controls/Fusion/impl/qtquickcontrols2fusionstyleimplplugin.lib",
    "qml/QtMultimedia/quickmultimediaplugin.lib",
];

/// The FFmpeg libraries of the static build, in PG_FFMPEG_STATIC.
const STATIC_FFMPEG_LIBS: &[&str] = &["avcodec", "avformat", "avutil", "swresample", "swscale"];

/// The Windows libraries the static Qt's libraries use (the manifest's non-Qt6 system entries).
const STATIC_QT_SYSTEM_LIBS: &[&str] = &[
    "Dwmapi", "Dxva2", "advapi32", "amstrmid", "authz", "avrt", "bcrypt", "comdlg32", "crypt32", "d2d1", "d3d11",
    "d3d12", "d3d9", "dnsapi", "dwrite", "dxgi", "dxguid", "evr", "gdi32", "icuin", "icuuc", "imm32", "iphlpapi",
    "kernel32", "ksuser", "mf", "mfcore", "mfplat", "mfreadwrite", "mfuuid", "mpr", "mscms", "ncrypt", "netapi32",
    "ntdll", "ole32", "oleaut32", "propsys", "runtimeobject", "secur32", "setupapi", "shcore", "shell32", "shlwapi",
    "strmiids", "synchronization", "uiautomationcore", "user32", "userenv", "uuid", "uxtheme", "version", "windowsapp",
    "winhttp", "winmm", "winspool", "wmcodecdspuuid", "ws2_32", "wtsapi32",
];

/// Whether the content key is set (non-empty), as CI sets it: the builder then emits no path lines for the sources.
fn has_content_key() -> bool {
    std::env::var("PG_DESKTOP_CONTENT_KEY").is_ok_and(|key| !key.is_empty())
}
