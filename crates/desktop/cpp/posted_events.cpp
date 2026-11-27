// The GUI thread's posted events, run now (src/bridges/shell_bridge.rs). Qt runs posted events in its event loop, so a
// call queued to the grid model's lane waits until the loop returns to it; a save that must see that call's result
// (the file shown) drains the queue first.

#include "posted_events.h"

#include <QtCore/QCoreApplication>

namespace pg {

void drain_posted_events() {
    QCoreApplication::sendPostedEvents(nullptr, 0);
}

}  // namespace pg
