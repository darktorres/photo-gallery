// The window's one deadline timer (S-12.18; the design passed spike S13). Re-arming restarts the one QTimer, so a later or an
// earlier deadline replaces the pending one, and a timeout that arrives after a re-arm is harmless: the core fires only the
// deadlines that are due.

#include "pg-desktop/cpp/deadline.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QElapsedTimer>
#include <QtCore/QTimer>

#include "pg-desktop/src/deadline.cxx.h"

namespace pg {

namespace {

QTimer* timer() {
    static QTimer* t = [] {
        auto* timer = new QTimer(QCoreApplication::instance());
        timer->setSingleShot(true);
        QObject::connect(timer, &QTimer::timeout, [] { deadline_fired(); });
        return timer;
    }();
    return t;
}

}  // namespace

void deadline_arm(int64_t ms) {
    timer()->start(static_cast<int>(ms < 0 ? 0 : ms));
}

void deadline_disarm() {
    timer()->stop();
}

int64_t deadline_now_ms() {
    static QElapsedTimer clock = [] {
        QElapsedTimer t;
        t.start();
        return t;
    }();
    return clock.elapsed();
}

}  // namespace pg
