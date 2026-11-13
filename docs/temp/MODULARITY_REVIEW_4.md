# Modularity review 4: what is left

The fourth review of [MODULARITY_PLAN.md](MODULARITY_PLAN.md) (2026-11-10) had steps M-21 to M-49. All of them are done in the code except the three items below. The finished steps are removed from this file; their full text is in git history (`git show e428ab2:docs/temp/MODULARITY_REVIEW_4.md`). The ground rules and the gates are the plan's.

## Rule still in use

- **R-1: an outcome a front end words is pg-model data.** `pg-client` depends on `pg-model` alone, so it cannot name a lower crate's type. Every outcome that is put into words is a plain data type in `pg-model`, built by the crate that has the outcome. `pg_client::text` words it, and both front ends render it.

## M-22: `gallery scan` says a refused mount

- **Found.** The scanner records a refused mount in the folder ledger (`FolderFailure::MountRefused`, the walk's `batch.failed` in `crates/scan/src/lib.rs`), and returns the first few as `ScanStats::refused_mount_list`. `gallery scan` never prints them. `scan_report` in `crates/cli/src/lib.rs` prints the counts, the scan notices and the kept failures, and none of these carries a refused mount. The test `gallery_scan_prints_its_notes` asserts the opposite, with the comment "A refused mount is the folder ledger's: no line". So a refused mount is skipped without a word on the command line, which is the bug M-22 names.
- **Fix.** `scan_report` prints one line for each entry of `refused_mount_list`: the mount, shown under the catalog's rules, and the reason worded by `pg_client::text` (`refused_reason`). The line is a `cli::Line` with its English in `crates/client/locales/en/cli.ftl`, not a `Notice`: the inbox already shows the ledger's refusal, and the CLI's own lines are the bundle's `cli` section. `refused_mounts` counts every refusal, and the list keeps only the first few, so when the count is higher than the list one more line says how many were not listed.
- **Test.** `gallery_scan_prints_its_notes` expects one line per listed mount, and the "not listed" line when the count is higher. `a_refused_folder_between_two_others_does_not_stop_the_scan` keeps passing.
- Gate: Windows `just test`, Linux checks.

## M-26: English words outside the bundle

- **Found.** Two pieces of English are still in code, not in the Fluent bundle, so the pseudo-locale's clipping check never sees them:
  - `crates/client/src/keys.rs`: the hint labels (22 constants, from `SELECT` "Click to select" to `CLOSE`), and the lines `grid_hint` and `viewer_hint` join from them.
  - `crates/desktop/src/gallery/model.rs` (`refresh_derived`): the status bar's selection line, `format!("{}   {symbol}", self.info(self.current))`.
- **Fix.** The hint labels move to `crates/client/locales/en/ui.ftl`, one message per label, and the joins become messages too (the separator is a message). The labels are worded through `pg_client::text`. The selection line's spacing becomes a message with the info and the symbol as its arguments. `keys.rs` keeps the key map and which keys each hint shows, and loses the words.
- Gate: Windows `just test`, `just shots` (the pseudo-locale).

## M-47: one-line wrappers came back

- **Found.** M-47 removed the one-line wrappers of `crates/client/src/text.rs`, so that each message has one API: the caller builds the `Message` and calls `render`. Five were added later, with the notifications' failure ledger, and remain: `failed_file_line`, `failed_folder_line`, `failed_folder_reason`, `failed_folder_group`, and `since_dismissed`. The last one also holds a rule, that a count of zero says nothing.
- **Fix.** Remove the five. Each caller builds its `Message` (`FailedFileLine`, `FailedFolderLine` with its `why` rendered from a `FailedFolderReason` message, `FailedFolderReason`, `FailedFolderGroup`, `LedgerSinceDismissed`) and calls `render`. The zero rule moves to the caller of `since_dismissed`. Find the callers with a search for the five names.
- Gate: Windows `just test`.

## Order

The three items are independent of one another, and each can be done alone.
