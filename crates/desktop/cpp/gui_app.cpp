// The GUI application's arguments, as Qt keeps them. The bytes are the ones `main` read (src/gui_app.rs), so the
// process's arguments are not read a second time.

#include "gui_app.h"

#include <QtCore/QByteArray>
#include <QtCore/QObject>
#include <QtCore/QString>
#include <QtCore/QStringList>
#include <QtGui/QFont>
#include <QtGui/QGuiApplication>
#include <QtGui/QImageReader>

#include <cstddef>
#include <vector>

namespace pg {

namespace {

#ifdef Q_OS_WIN
// The platform Qt will pick for this run: a -platform or --platform argument (in either its separate or its `=` form),
// else QT_QPA_PLATFORM. Qt reads the arguments first, so an argument wins over the variable. The name is what precedes
// any `:` option.
QString requested_platform(int argc, char** argv) {
    QStringList args;
    for (int i = 1; i < argc; ++i)
        args.append(QString::fromLocal8Bit(argv[i]));
    QString name;
    for (int i = 0; i < args.size() && name.isEmpty(); ++i) {
        const QString& arg = args[i];
        if (arg == "-platform" || arg == "--platform")
            name = i + 1 < args.size() ? args[i + 1] : QString();
        else if (arg.startsWith("-platform="))
            name = arg.mid(QStringLiteral("-platform=").size());
        else if (arg.startsWith("--platform="))
            name = arg.mid(QStringLiteral("--platform=").size());
    }
    if (name.isEmpty())
        name = qEnvironmentVariable("QT_QPA_PLATFORM");
    return name.section(QLatin1Char(':'), 0, 0);
}
#endif

// The offscreen platform (the test hooks) draws text with FreeType on Windows. FreeType reads the fonts from
// QT_QPA_FONTDIR, or else from <Qt>/lib/fonts, which a Windows Qt install doesn't have: the text would draw as empty
// boxes. So when the platform is offscreen and no font folder is given, the Windows fonts folder is used. It is set here
// because Qt reads it when the application's font database is made, and only the C runtime's environment is read by Qt.
void use_windows_fonts_for_offscreen(int argc, char** argv) {
#ifdef Q_OS_WIN
    if (requested_platform(argc, argv) != "offscreen" || qEnvironmentVariableIsSet("QT_QPA_FONTDIR"))
        return;
    QString windir = qEnvironmentVariable("WINDIR");
    if (windir.isEmpty())
        windir = QStringLiteral("C:/Windows"); // the same fallback as tools/qmltest.py
    qputenv("QT_QPA_FONTDIR", (windir + "/Fonts").toLocal8Bit());
#endif
}

// Qt keeps the argc and argv it is given for the application's whole life, so they are a child of the application,
// as cxx-qt-lib keeps its own.
class Arguments : public QObject {
public:
    explicit Arguments(rust::Slice<const std::uint8_t> args) : m_text(args.begin(), args.end()) {
        for (std::size_t i = 0; i < m_text.size();) {
            m_argv.push_back(m_text.data() + i);
            while (i < m_text.size() && m_text[i] != 0)
                ++i;
            ++i;  // past the NUL
        }
        m_argc = static_cast<int>(m_argv.size());
        m_argv.push_back(nullptr);
    }

    int& argc() { return m_argc; }
    char** argv() { return m_argv.data(); }

private:
    // Not changed after the constructor: the pointers into it stay valid.
    std::vector<char> m_text;
    std::vector<char*> m_argv;
    int m_argc = 0;
};

// The offscreen platform has no system font to ask for: its theme's default is the generic "Sans Serif" (the Quick
// Controls take it, through the platform theme, for every control's font), and its font database on Windows can't
// resolve that name, so Qt falls back to the first installed family (Agency FB, a very condensed face). The windowed
// platform takes the user's message font (Segoe UI). So "Sans Serif" is substituted with Segoe UI here, and the
// screenshots and the QML tests draw the face users see. A substitution, not a font set on the application: the
// Quick Controls don't read the application font.
void use_segoe_for_offscreen() {
#ifdef Q_OS_WIN
    if (QGuiApplication::platformName() != "offscreen")
        return;
    QFont::insertSubstitution(QStringLiteral("Sans Serif"), QStringLiteral("Segoe UI"));
#endif
}

}  // namespace

std::unique_ptr<QGuiApplication> new_gui_application(rust::Slice<const std::uint8_t> args) {
    // Qt's default image allocation limit (256 MB) rejects a full-size original over about 67 MP, such as a 10000 x 7514
    // JPEG that Yandex serves: its decode needs 4 bytes a pixel (287 MB). 1024 MB admits up to about 268 MP.
    QImageReader::setAllocationLimit(1024);
    auto* arguments = new Arguments(args);
    use_windows_fonts_for_offscreen(arguments->argc(), arguments->argv());
    auto app = std::make_unique<QGuiApplication>(arguments->argc(), arguments->argv());
    arguments->setParent(app.get());
    use_segoe_for_offscreen();
    return app;
}

}  // namespace pg
