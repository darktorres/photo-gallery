# dhat-rs 0.3.3, patched

This is [dhat-rs](https://github.com/nnethercote/dhat-rs) 0.3.3 (MIT OR
Apache-2.0, by Nicholas Nethercote), changed so that heap profiles with whole
backtraces (`trim_backtraces(None)`) take minutes rather than hours:

- **Backtraces are taken before the lock** when they are untrimmed, so threads
  unwind in parallel.
- **Frame pointers are walked** instead of DWARF unwind tables when the build
  passes `-C force-frame-pointers=yes` on x86_64 or aarch64, on Linux or macOS
  (`build.rs`). The walk reads `rbp` or `x29`, and bounds itself with the
  thread's stack from `pthread_getattr_np` on Linux and from
  `pthread_get_stackaddr_np` and `pthread_get_stacksize_np` on macOS. Other
  builds, Windows included, unwind with `backtrace` as before.
- **A backtrace is its return addresses.** Each distinct address is resolved
  once, when the profile is written.
- **The global peak is recorded lazily.** Each program point copies its counts
  at the peak before its next change, instead of every program point being
  updated at each new peak.
- **The lock is a futex** (`std::sync::Mutex`), where `mintex` spins.

The output format is unchanged. The upstream README follows.

# dhat-rs

**Warning:** *This crate is experimental. It relies on implementation techniques
that are hard to keep working for 100% of configurations. It may work fine for
you, or it may crash, hang, or otherwise do the wrong thing. Its maintenance is
not a high priority of the author. Support requests such as issues and pull
requests may receive slow responses, or no response at all. Sorry!*

This crate provides heap profiling and ad hoc profiling capabilities to Rust
programs, similar to those provided by [DHAT].

[DHAT]: https://www.valgrind.org/docs/manual/dh-manual.html

It also provides heap usage testing capabilities, which let you write tests
that check things like:
- "This code should do exactly 96 heap allocations".
- "The peak heap usage of this code should be less than 10 MiB".
- "This code should free all heap allocations before finishing".

It provides helpful details if these fail.

See the [crate documentation] for details on how to use it.

[crate documentation]: https://docs.rs/dhat

## License

Licensed under either of
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or
  http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
