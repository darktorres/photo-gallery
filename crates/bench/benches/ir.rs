//! Instruction counts (callgrind, through gungraun) of the per-file work of the import and of the thumbnail task, and
//! of the viewer's image (a still's decode and fit, a video's poster), the catalog's, and the pairing pass's join.
//! Valgrind is Linux-only, so these run in the Linux container: `just perf`. On other
//! systems the target's `main` only prints where to run them, so `cargo clippy --all-targets` still builds it.

#[cfg(target_os = "linux")]
#[expect(
    unreachable_pub,
    reason = "gungraun's `library_benchmark` makes a `pub mod` for each benchmark; the harness is not a library, so none is reachable"
)]
mod benches {
    include!("ir/linux.rs");
}

#[cfg(target_os = "linux")]
use benches::*;
#[cfg(target_os = "linux")]
use gungraun::prelude::*;

#[cfg(target_os = "linux")]
main!(library_benchmark_groups = per_file, viewer, catalog, scan, removal, pairing);

#[cfg(not(target_os = "linux"))]
#[expect(clippy::print_stderr, reason = "the stub tells the user where the benches run")]
fn main() {
    eprintln!("the callgrind benches run on Linux, in the container: `just perf`");
}
