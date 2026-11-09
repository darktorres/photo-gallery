// The window's one deadline timer (S-12.18): a single-shot QTimer that the controller's next deadline arms. cxx-qt-lib 0.10 has
// no QTimer, so this is the glue (as cpp/screenshot.cpp is). Only the GUI thread touches it.
#pragma once

#include <cstdint>

class QObject;

namespace pg {

// Fire the owner's `timeout` invokable in `ms` milliseconds, replacing any pending arm (earlier or later). The timer is a child
// of `owner` (the shell), so it is destroyed with it.
void deadline_arm(QObject* owner, int64_t ms);
// Cancel the pending arm of `owner`'s timer, if any.
void deadline_disarm(QObject* owner);
// The monotonic clock the deadlines are on (milliseconds since the first call).
int64_t deadline_now_ms();

}  // namespace pg
