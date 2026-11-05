// The window's one deadline timer (S-12.18): a single-shot QTimer that the core's next deadline arms. cxx-qt-lib 0.10 has
// no QTimer, so this is the glue (as cpp/screenshot.cpp is). Only the GUI thread touches it.
#pragma once

#include <cstdint>

namespace pg {

// Fire deadline_fired() in `ms` milliseconds, replacing any pending arm (earlier or later).
void deadline_arm(int64_t ms);
// Cancel the pending arm, if any.
void deadline_disarm();
// The monotonic clock the deadlines are on (milliseconds since the first call).
int64_t deadline_now_ms();

}  // namespace pg
