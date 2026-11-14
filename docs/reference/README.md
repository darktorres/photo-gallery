# Reference

Files here hold facts that stay true: measurements, how the system behaves, and decisions already made. A file
goes here only when it has no tasks. Plans stay in [`docs/plans/`](../plans/) or [`docs/temp/`](../temp/), and
sources (papers, the references ledger) stay in [`research/`](../research/).

- [STORAGE.md](STORAGE.md): drive kinds, file systems that are refused, and file identity on Linux.
- [HEAP_PROFILE.md](HEAP_PROFILE.md): heap totals of the scan, the import and the passes, before and after three rounds of
  fixes (dhat, 2026-11-01 and 2026-11-02). The profiles are no longer on disk.
- [IO.md](IO.md): measured HDD read orders and buffer sizes for the import, how paths over 260 characters are handled, and how folders with dots or spaces at the end of their names are handled.
- [START_PROFILE.md](START_PROFILE.md): where the app start goes, thread by thread, and the startup before and after the thumbnail cache changes.
- [REVERSE_SEARCH.md](REVERSE_SEARCH.md): Yandex reverse image search: what the results contain, why a plain HTTP client gets fewer, and what the port reads, with its open questions.
- [REFACTOR_DESIGN.md](REFACTOR_DESIGN.md): the crate refactor as built: the design decisions, where the build differs from the first plan, and the names to reuse. Open work is in `docs/plans/REFACTOR_LEDGER.md`.
- [PROFILING.md](PROFILING.md): how the app's CPU and heap cost is measured (the tools, the callgrind benches and the
  budgets), and the measurements: the start, the library scan, `analyze` and `thumbs::make`.
