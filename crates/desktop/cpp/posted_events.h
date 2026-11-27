// The GUI thread's posted events, run now (src/bridges/shell_bridge.rs). The grid model's commands are queued to its lane
// as posted events, so a command still in the queue has not happened yet until they are drained.
#pragma once

namespace pg {

// Runs the posted events of the calling thread now, the queued model commands among them, and returns when they are done.
// Call on the GUI thread.
void drain_posted_events();

}  // namespace pg
