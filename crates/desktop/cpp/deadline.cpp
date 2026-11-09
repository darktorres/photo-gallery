// The window's one deadline timer (S-12.18; the design passed spike S13 in docs/REFACTOR_PLAN.md). Re-arming restarts the owner's
// one QTimer, so a later or an earlier deadline replaces the pending one, and a timeout that arrives after a re-arm is harmless:
// the controller fires only the deadlines that are due.

#include "pg-desktop/cpp/deadline.h"

#include <QtCore/QElapsedTimer>
#include <QtCore/QMetaObject>
#include <QtCore/QObject>
#include <QtCore/QString>
#include <QtCore/QTimer>

namespace pg {

namespace {

// The timer of `owner`, made on the first arm as a child of the owner. Its timeout queues a call to the owner's `timeout`
// invokable, so the shell's handler runs as an ordinary event.
QTimer* timer_of(QObject* owner) {
    static const QString name = QStringLiteral("pg-deadline");
    if (auto* found = owner->findChild<QTimer*>(name, Qt::FindDirectChildrenOnly)) {
        return found;
    }
    auto* timer = new QTimer(owner);
    timer->setObjectName(name);
    timer->setSingleShot(true);
    QObject::connect(timer, &QTimer::timeout, owner, [owner] {
        QMetaObject::invokeMethod(owner, "timeout", Qt::QueuedConnection);
    });
    return timer;
}

}  // namespace

void deadline_arm(QObject* owner, int64_t ms) {
    timer_of(owner)->start(static_cast<int>(ms < 0 ? 0 : ms));
}

void deadline_disarm(QObject* owner) {
    timer_of(owner)->stop();
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
