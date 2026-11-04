// The GUI application's arguments, as Qt keeps them. The bytes are the ones `main` read (src/gui_app.rs), so the
// process's arguments are not read a second time.

#include "gui_app.h"

#include <QtCore/QByteArray>
#include <QtCore/QObject>
#include <QtCore/QString>

#include <cstddef>
#include <vector>

namespace pg {

namespace {

// The offscreen platform (the test hooks) draws text with FreeType on Windows. FreeType reads the fonts from
// QT_QPA_FONTDIR, or else from <Qt>/lib/fonts, which a Windows Qt install doesn't have: the text would draw as empty
// boxes. So when the platform is offscreen and no font folder is given, the Windows fonts folder is used. It is set here
// because Qt reads it when the application's font database is made, and only the C runtime's environment is read by Qt.
void use_windows_fonts_for_offscreen() {
#ifdef Q_OS_WIN
    if (qEnvironmentVariable("QT_QPA_PLATFORM") != "offscreen" || qEnvironmentVariableIsSet("QT_QPA_FONTDIR"))
        return;
    const QString windir = qEnvironmentVariable("WINDIR");
    if (!windir.isEmpty())
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

}  // namespace

std::unique_ptr<QGuiApplication> new_gui_application(rust::Slice<const std::uint8_t> args) {
    use_windows_fonts_for_offscreen();
    auto* arguments = new Arguments(args);
    auto app = std::make_unique<QGuiApplication>(arguments->argc(), arguments->argv());
    arguments->setParent(app.get());
    return app;
}

}  // namespace pg
