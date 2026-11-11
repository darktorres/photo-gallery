//! Instruction counts (callgrind, through gungraun) of the per-file work of the import and of the thumbnail task, and
//! of the viewer's image (a still's decode and fit, a video's poster).
//! Valgrind is Linux-only, so these run in the Linux container: `just perf` (docs/PROFILING.md). On other
//! systems the target's `main` only prints where to run them, so `cargo clippy --all-targets` still builds it.

#[cfg(target_os = "linux")]
include!("ir/linux.rs");

#[cfg(not(target_os = "linux"))]
#[expect(clippy::print_stderr, reason = "the stub tells the user where the benches run")]
fn main() {
    eprintln!("the callgrind benches run on Linux, in the container: `just perf` (docs/PROFILING.md)");
}
