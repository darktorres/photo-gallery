// The window icon. Windows shows the exe's icon in Explorer only: a window's title bar and taskbar button take the icon
// the application gives them, so the PNG (crates/ui/resources/app-icon.png) is set here as the application's icon.

#include "app_icon.h"

#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>

namespace pg {

void set_app_icon() {
    // build.rs puts the resource under the QML module's prefix (qrc:/qt/qml/pg/gallery/).
    QGuiApplication::setWindowIcon(QIcon(QStringLiteral(":/qt/qml/pg/gallery/resources/app-icon.png")));
}

}  // namespace pg
