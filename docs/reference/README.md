# Reference

Files here hold facts that stay true: measurements, how the system behaves, and decisions already made. A file
goes here only when it has no tasks. Plans stay in [`docs/plans/`](../plans/) or [`docs/temp/`](../temp/), and
sources (papers, the references ledger) stay in [`research/`](../research/).

- [STORAGE.md](STORAGE.md): drive kinds, file systems that are refused, and file identity on Linux.
- [HEAP_PROFILE.md](HEAP_PROFILE.md): heap totals of the scan, the import and the passes, before and after three rounds of
  fixes (dhat, 2026-11-01 and 2026-11-02). The profiles are no longer on disk.
- [IO.md](IO.md): measured HDD read orders and buffer sizes for the import, how paths over 260 characters are handled, and how folders with dots or spaces at the end of their names are handled.
- [START_PROFILE.md](START_PROFILE.md): where the app start goes, thread by thread, and the startup before and after the thumbnail cache changes.
