// The QML test host (S-14.3, `just qmltest`): Qt Quick Test's own main, as qmltestrunner is, with the image providers the
// app registers in C++ (crates/desktop/src/lib.rs: thumb, thumbhash, full). The app's providers decode the catalog's
// files through Rust, which the tests don't have, so these stand in: each one makes a plain image of the size asked for,
// and the image:// URLs the tests pass resolve as they do in the app.
#include <QtCore/QObject>
#include <QtCore/QResource>
#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtCore/qtenvironmentvariables.h>
#include <QtGui/QColor>
#include <QtGui/QImage>
#include <QtQml/QQmlEngine>
#include <QtQuick/QQuickImageProvider>
#include <QtQuickTest/quicktest.h>

namespace {

// One stand-in provider: a solid image of the requested size, or 16 px square when none is asked for (the placeholder
// is requested without a size).
class StubProvider : public QQuickImageProvider {
public:
    StubProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}

    QImage requestImage(const QString& id, QSize* size, const QSize& requestedSize) override {
        const QSize s = requestedSize.isValid() ? requestedSize : QSize(16, 16);
        if (size) *size = s;
        QImage image(s, QImage::Format_RGB32);
        image.fill(QColor::fromHsv(int(qHash(id) % 360), 120, 200));
        return image;
    }
};

// The test setup: the providers go into the engine before the tests load their QML.
class Setup : public QObject {
    Q_OBJECT
public slots:
    // The mascots' resource bundle (tools/qmltest.py): the app has these files in its resources, and Mascot.qml loads
    // them by qrc:, so the bundle goes in before any test loads that QML. A bundle that does not load fails the run.
    void applicationAvailable() {
        const QString bundle = qEnvironmentVariable("PG_QML_RESOURCES");
        if (!bundle.isEmpty() && !QResource::registerResource(bundle))
            qFatal("the resource bundle %s did not load", qPrintable(bundle));
    }

    void qmlEngineAvailable(QQmlEngine* engine) {
        for (const char* name : {"thumb", "thumbhash", "full"})
            engine->addImageProvider(QString::fromLatin1(name), new StubProvider);
    }

    void cleanupTestCase() {}
};

}  // namespace

QUICK_TEST_MAIN_WITH_SETUP(qmltestrunner, Setup)

#include "main.moc"
