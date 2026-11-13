# Notifications: open items

Status (2026-11-12): the notifications feature is built (S1 to S3, then [NOTIFICATIONS_FIX_PLAN.md](NOTIFICATIONS_FIX_PLAN.md)).
The reference for the code is [NOTIFICATIONS.md](NOTIFICATIONS.md) in this folder. This file lists only what is still
open, from an audit of the plan against the code. The audit read the working tree, which has uncommitted edits in
`notices.rs`, `keys.rs`, `text.rs`, `failed_files.rs` and `message.rs`; those edits are not reviewed here.

## Behaviour

1. **Progress poke.** While `failed > 0`, the desktop's change pump should poke the inbox on `Change::Progress`, so the
   "N failed" link follows within about 1 s. Today `on_pass` (`crates/desktop/src/library.rs`) only calls
   `refresh_progress`.
2. **Crop failures.** `crop::Stats` has no failure list (`crates/crop/src/lib.rs`, no `keep_failure`), so
   `Notice::Cropped` carries none. Add `keep_failure` to `crop::Stats`, carry the failures into `Cropped`, and add the
   `refused` field the plan names (`crates/model/src/notice.rs`, `Cropped`).
3. **Retired `root-refused` rows.** `convert_notify` deletes only `scan-unreadable` and `mount-refused`
   (`crates/catalog/src/convert.rs:127`). Add `root-refused`, or those rows decode as `Unknown` forever.
4. **`PassFailed`.** The "N failed" split is in the status bar, so retire `Message::PassFailed`, its arm in `pass_status`
   (`crates/client/src/text.rs:265`) and the `pass-failed` entry (`crates/client/locales/en/text.ftl:72`).

## Tests

1. **QML stubs** (`crates/desktop/tests/qml/stubs/pg/gallery/`): add `Notices.qml` and `LogModel.qml` with their `qmldir`
   lines, `devMode` in `Settings.qml` and `globalAction` in `Shell.qml`. Run `just qmltest` first to see which tests fail
   without them.
2. **Missing tests:**
   - `THUMBS_VERSION` is nonzero, against the real constant (`crates/library/src/lib.rs`).
   - `notice_post` dedup ignores `detail` (`crates/catalog/src/inbox.rs`). See the design item on dedup below.
   - Every trigger fires: a test inserts through each (`crates/catalog/src/schema.rs` only checks they exist).
   - The roots trigger fires from the CLI's `remove` (only a catalog-level test exists).
   - `markRead` with a displayed serial leaves later rows new (only the newest path is covered).
   - `crates/catalog/src/fixture/raw.rs`: `older_thumbs_version` / `start_thumbs_version` beside the import and borders
     pairs.

## Performance gate

1. `just perf` against `docs/perf/baseline.json` has not run since the thumbs-pass restructure. The baseline predates it.

## Design: code and the design disagree

Each row needs a decision: change the code or change `NOTIFICATIONS.md`.

| Topic | Design says | Code does |
|---|---|---|
| `Notices` API | `markRead`, `reveal(fileId)`, `perform(action)` | `setViewing`; `Shell.reveal`; one method per action |
| Dev-mode property | `devMode` on `settings_bridge.rs` | Read-only `Shell.devMode`, saved by `Command::SaveDevMode` |
| Keeper-close failure | Log only (`WalNotMerged`) | Stored as `CatalogNotClosed` (`crates/app/src/app.rs`) |
| Dismissed events | Kept until purged after 30 days | Row deleted at dismissal (`crates/catalog/src/inbox.rs`) |
| Stored-notice dedup | Summary fields only | Whole JSON, `detail` included (`crates/catalog/src/inbox.rs`) |
| `IMPORT_VERSION` bump | Re-badges the import entry once | Test asserts it does not (`crates/catalog/src/ledger.rs`) |
| Root removal cleanup | Trigger on `roots` | Foreign key `ON DELETE CASCADE` (same effect) |
| Cache events | Five variants, `into_notice` | Seven variants (`TouchesWritten`, `Stored`), `Event::effect` |
| `Panels.modals` | `ArrayVec<PanelKind, 8>` | `Vec<PanelKind>` (`crates/client/src/controller/panels.rs`) |
