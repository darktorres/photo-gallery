// The --screenshot test hook. The window's frame as shown is read back (QQuickWindow::grabWindow), popups and menus
// included, at the screen's device pixel ratio. The frame is taken when the window has settled, not after a fixed
// delay: QML says it is `ready` (Main.qml: the first list shown and the test hooks run), no image in the window is
// still loading, no one-shot QML Timer is pending (each one is a change the window has yet to make), no animation runs
// on a visible item (a busy indicator spins), the library has no job running and no write into the files pending, and
// no frame has been drawn for a while. `max_ms` is the most the hook waits; a window that never settles is saved anyway, and
// the settle log says why. The log is written beside the image: `<image>.settle.txt`.

#include "screenshot.h"

#include <atomic>
#include <memory>

#include <QtCore/QCoreApplication>
#include <QtCore/QElapsedTimer>
#include <QtCore/QFile>
#include <QtCore/QTextStream>
#include <QtCore/QTimer>
#include <QtGui/QImage>
#include <QtQuick/QQuickWindow>

#include <cstdio>

namespace pg {

namespace {

// How often the window is checked.
constexpr int kPollMs = 50;
// The window is idle once no frame has been drawn for this long.
constexpr int kQuietMs = 300;

QQuickWindow* window_of(QQmlApplicationEngine& engine) {
    for (QObject* root : engine.rootObjects())
        if (auto* window = qobject_cast<QQuickWindow*>(root))
            return window;
    return nullptr;
}

// What the window holds and what keeps it busy. Images are checked through their base class (QQuickImageBase,
// status Loading = 2); a one-shot QML Timer that is running (`repeat` false) is a change still to come. The counts of
// everything seen go into the settle log, so a check that sees nothing shows up there.
struct Busy {
    int images = 0;
    int loading = 0;
    int timers = 0;
    int pending = 0;
    // Running animations of visible items: a busy indicator spins, a transition is under way.
    int animating = 0;
    QStringList animating_owners;
    // The library: its job (scanning, importing, thumbnails, borders) is running, and its writer has ratings or turns
    // still to write into the files (or a failure that stops them: that stays, and the log says so).
    int jobs = 0;
    int writes = 0;
    // Shell's pauses still to run (Shell.pending): a debounce, the session, the layout or the thumbnails to come.
    int deadlines = 0;

    bool idle() const {
        return loading == 0 && pending == 0 && animating == 0 && jobs == 0 && writes == 0 && deadlines == 0;
    }
    QString text() const {
        QString text = QStringLiteral("%1 image(s) seen, %2 loading; %3 timer(s) seen, %4 pending; %5 animating; "
                                      "library job running: %6, writes pending: %7; core pauses pending: %8")
                           .arg(images).arg(loading).arg(timers).arg(pending).arg(animating).arg(jobs).arg(writes)
                           .arg(deadlines);
        if (!animating_owners.isEmpty())
            text += " (" + animating_owners.join(", ") + ")";
        return text;
    }
};

Busy busy_of(QQuickWindow* window) {
    Busy busy;
    // The window's core (Main.qml's `shellCore`): its pending pauses are asked through its invokable.
    if (QObject* core = window->property("shellCore").value<QObject*>()) {
        bool pending = false;
        if (QMetaObject::invokeMethod(core, "pending", Q_RETURN_ARG(bool, pending)) && pending)
            ++busy.deadlines;
    }
    for (QObject* object : window->findChildren<QObject*>()) {
        if (object->inherits("QQuickImageBase")) {
            ++busy.images;
            if (object->property("status").toInt() == 2)
                ++busy.loading;
        } else if (object->inherits("QQmlTimer")) {
            ++busy.timers;
            if (object->property("running").toBool() && !object->property("repeat").toBool())
                ++busy.pending;
        } else if (object->inherits("QQuickAbstractAnimation") && object->property("running").toBool()) {
            // The item that owns it, when it is an item that shows.
            QObject* owner = object->parent();
            if (owner && owner->inherits("QQuickItem") && owner->property("visible").toBool()) {
                ++busy.animating;
                busy.animating_owners << QString::fromLatin1(owner->metaObject()->className());
            }
        } else if (object->property("writeStatus").isValid()) {
            // The Library object (the only one with a write status): its job and its writer.
            if (object->property("busy").toBool())
                ++busy.jobs;
            if (!object->property("writeStatus").toString().isEmpty())
                ++busy.writes;
        }
    }
    return busy;
}

void write_log(const QString& path, const QString& text) {
    QFile file(path + QStringLiteral(".settle.txt"));
    if (file.open(QIODevice::WriteOnly | QIODevice::Text)) {
        QTextStream(&file) << text << '\n';
    }
}

// The render thread stamps the last frame drawn; the poll (on the GUI thread) reads it.
struct Settle {
    QElapsedTimer clock;      // the monotonic clock both sides read (elapsed() is thread-safe)
    std::atomic<qint64> last_frame_ms{0};
    QElapsedTimer waited;     // since the window said it was ready
    QElapsedTimer total;      // since the hook was scheduled
    bool ready_seen = false;
    bool hooked = false;
};

}  // namespace

void save_screenshot_later(QQmlApplicationEngine& engine, const QString& path, int max_ms) {
    auto state = std::make_shared<Settle>();
    state->clock.start();
    state->total.start();
    // Polled: the window's `ready` is a QML property, and this file has no moc for a slot to connect.
    auto* poll = new QTimer(&engine);
    poll->setInterval(kPollMs);
    QObject::connect(poll, &QTimer::timeout, &engine, [&engine, poll, path, max_ms, state] {
        QQuickWindow* window = window_of(engine);
        if (!window)
            return;
        const QVariant ready = window->property("ready");
        // A window without the property is ready as it is.
        if (ready.isValid() && !ready.toBool())
            return;
        if (!state->ready_seen) {
            state->ready_seen = true;
            state->waited.start();
        }
        if (!state->hooked) {
            state->hooked = true;
            // Direct: this runs on the render thread when the scene is threaded; only the atomic is touched.
            QObject::connect(window, &QQuickWindow::afterRendering, window, [state] {
                state->last_frame_ms.store(state->clock.elapsed(), std::memory_order_relaxed);
            }, Qt::DirectConnection);
        }
        const qint64 quiet = state->clock.elapsed() - state->last_frame_ms.load(std::memory_order_relaxed);
        const Busy busy = busy_of(window);
        const bool settled = busy.idle() && quiet >= kQuietMs;
        const bool timed_out = state->total.elapsed() >= max_ms;
        if (!settled && !timed_out)
            return;
        poll->stop();
        poll->deleteLater();
        const QString report = settled
            ? QStringLiteral("settled after %1 ms (%2 ms since ready, quiet for %3 ms); %4")
                  .arg(state->total.elapsed()).arg(state->waited.elapsed()).arg(quiet).arg(busy.text())
            : QStringLiteral("timed out after %1 ms (%2 ms since ready, quiet for %3 ms); %4")
                  .arg(state->total.elapsed()).arg(state->waited.elapsed()).arg(quiet).arg(busy.text());
        if (!settled)
            std::fprintf(stderr, "screenshot: the window did not settle: %s\n", qUtf8Printable(report));
        write_log(path, report);
        if (QQuickWindow* grab = window_of(engine))
            if (!grab->grabWindow().save(path))
                std::fprintf(stderr, "screenshot: can't save %s\n", qUtf8Printable(path));
        QCoreApplication::quit();
    });
    poll->start();
}

}  // namespace pg
