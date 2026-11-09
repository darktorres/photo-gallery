// The static Qt build (PG_QT_STATIC set, docs/STATIC_BUILD.md): a static Qt loads no plugin from disk, so each plugin and
// QML plugin the app uses is registered here, by its class name (the qmldir "classname" and the plugin's own name).
// build.rs compiles this file only for the static build. gui_app.rs calls pg_static_qt_link (below) under the
// pg_static_qt cfg, so this object, with the registrations, is linked in even though nothing else names it.
#include <QtPlugin>
#include <QtCore/qtresource.h>
#include <QtQml/qqmlextensionplugin.h>

// Platform, image formats, TLS, media and icon plugins.
Q_IMPORT_PLUGIN(QWindowsIntegrationPlugin)
Q_IMPORT_PLUGIN(QOffscreenIntegrationPlugin)
Q_IMPORT_PLUGIN(QGifPlugin)
Q_IMPORT_PLUGIN(QICNSPlugin)
Q_IMPORT_PLUGIN(QICOPlugin)
Q_IMPORT_PLUGIN(QJpegPlugin)
Q_IMPORT_PLUGIN(QSvgPlugin)
Q_IMPORT_PLUGIN(QTgaPlugin)
Q_IMPORT_PLUGIN(QTiffPlugin)
Q_IMPORT_PLUGIN(QWbmpPlugin)
Q_IMPORT_PLUGIN(QWebpPlugin)
Q_IMPORT_PLUGIN(QSchannelBackend)
Q_IMPORT_PLUGIN(QTlsBackendCertOnly)
Q_IMPORT_PLUGIN(QFFmpegMediaPlugin)
Q_IMPORT_PLUGIN(QWindowsMediaPlugin)
Q_IMPORT_PLUGIN(QSvgIconPlugin)

// QML plugins: the modules Main.qml and the other components import (qml/*.qml), and what they import in turn.
Q_IMPORT_QML_PLUGIN(QtQmlPlugin)
Q_IMPORT_QML_PLUGIN(QtQmlModelsPlugin)
Q_IMPORT_QML_PLUGIN(QtQmlWorkerScriptPlugin)
Q_IMPORT_QML_PLUGIN(QtQuick2Plugin)
Q_IMPORT_QML_PLUGIN(QtQuick_WindowPlugin)
Q_IMPORT_QML_PLUGIN(QtQuickLayoutsPlugin)
Q_IMPORT_QML_PLUGIN(QtQuickTemplates2Plugin)
Q_IMPORT_QML_PLUGIN(QtQuickControls2Plugin)
Q_IMPORT_QML_PLUGIN(QtQuickControls2ImplPlugin)
Q_IMPORT_QML_PLUGIN(QtQuickControls2BasicStylePlugin)
Q_IMPORT_QML_PLUGIN(QtQuickControls2BasicStyleImplPlugin)
Q_IMPORT_QML_PLUGIN(QtQuickControls2FusionStylePlugin)
Q_IMPORT_QML_PLUGIN(QtQuickControls2FusionStyleImplPlugin)
Q_IMPORT_QML_PLUGIN(QMultimediaQuickModule)

// Called from gui_app.rs (static build only), so this object, with the registrations above, is always linked. It also
// registers the Quick Controls' resources: the .qml files of the styles (ApplicationWindow.qml and the rest) and their
// compiled caches. A shared Qt registers them when its plugin DLLs load; a static one runs no initializer by itself,
// and the plugin libraries only reference them, so they are run here.
// The initializers are C++ functions (their declarations come from the macro, so they must not sit in the extern "C"
// function below).
static void init_quick_controls_resources() {
    Q_INIT_RESOURCE(qmake_QtQuick_Controls);

    Q_INIT_RESOURCE(qmake_QtQuick_Controls_impl);
    Q_INIT_RESOURCE(qmlcache_QuickControls2Impl);
    Q_INIT_RESOURCE(QuickControls2Impl_raw_qml_0);

    Q_INIT_RESOURCE(qmake_QtQuick_Controls_Basic);
    Q_INIT_RESOURCE(qmlcache_QuickControls2Basic);
    Q_INIT_RESOURCE(QuickControls2Basic_raw_qml_0);
    Q_INIT_RESOURCE(qtquickcontrols2basicstyle);

    Q_INIT_RESOURCE(qmake_QtQuick_Controls_Basic_impl);
    Q_INIT_RESOURCE(qmlcache_QuickControls2BasicStyleImpl);
    Q_INIT_RESOURCE(QuickControls2BasicStyleImpl_raw_qml_0);

    Q_INIT_RESOURCE(qmake_QtQuick_Controls_Fusion);
    Q_INIT_RESOURCE(qmlcache_QuickControls2Fusion);
    Q_INIT_RESOURCE(QuickControls2Fusion_raw_qml_0);
    Q_INIT_RESOURCE(qtquickcontrols2fusionstyle);

    Q_INIT_RESOURCE(qmake_QtQuick_Controls_Fusion_impl);
    Q_INIT_RESOURCE(qmlcache_QuickControls2FusionStyleImpl);
    Q_INIT_RESOURCE(QuickControls2FusionStyleImpl_raw_qml_0);
}

extern "C" void pg_static_qt_link() {
    init_quick_controls_resources();
}
