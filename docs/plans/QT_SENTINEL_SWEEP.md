# Qt sentinel sweep: `-1` and `-1.0` at the Rust↔QML boundary

**Status:** not started. This plan was split out of item 1 of `docs/plans/TYPE_MISMATCH_VALIDATION.md`, which now covers only the saved last file. Related: `docs/plans/BOUNDED_MEMORY_PLAN.md` Phase 7 step 7.9 (the item selection design) and its decision D29 (line 131: file ids as decimal strings); item 4 of `TYPE_MISMATCH_VALIDATION.md` (counts and positions).

## The problem

The Qt side marks "none" with `-1` or `-1.0`, in Rust and in QML. A sentinel number is one more value a receiver can mistake for a real index, and the convention is spread over both sides with nothing in the types to show it. A typed `int` receiver also takes an absent value as 0, so a missing value and a first row look the same there (measured below).

## Sites

Production code in `crates/desktop/src`, found by grepping for `-1` and `-2` at `1fa96de2`. Test code is excluded. The grep is not guaranteed complete.

- Rust state and models: `bridges/gallery_state.rs:77-80` (current item, viewing, anchor) and `:94-97` (rejected-edge fractions, `-1.0`); `bridges/log_model_state.rs:33-34` (selected row and seq); `bridges/notices_state.rs:51, 53` (selected row and pass); `log_model.rs:210, 211, 213`; `notices.rs:248, 568, 589, 628`; `lane_model.rs:52, 362` (the crop's number, and a failed pass); `ledgers.rs:10` (a ledger's code); `folders.rs:41, 257, 272` (`-1` for collapsed, `-1` for not in the tree, and a row fallback); `failed_files.rs:78` (`-1.0` for a folder's row); `gallery/items.rs:27, 150, 152`; `gallery/model.rs:159` (`set_anchor(-1)`); `gallery/nav.rs:8, 10`.
- Codes: `keymap.rs:127, 138, 155` (action codes); `command_line.rs:448, 453` (the mascot and the preview).
- Other: `yandex.rs:95` (the preview item); `video_player.rs:17` (the player handle, passed to `play_file`, `bridges/video_player_bridge.rs:24`); `shell.rs:64` (`to_i32`'s fallback); `rows.rs:27` (`qt_int`, whose sites belong to item 4).
- QML: about 48 lines (comments included) compare against `-1`, in `GalleryGrid.qml`, `InboxPanel.qml`, `LaneRow.qml`, `LogPanel.qml`, `Main.qml`, `Mascot.qml`, `RatingBar.qml`, `RemoveFolderDialog.qml`, `TestHooks.qml`, `Viewer.qml`, `YandexPanel.qml` and `YandexPreview.qml`. Each changes with the bridge that feeds it.

`settings.rs` has no `FileId` conversion and no `last_file`. The boundary for file ids is planned in the bridge that holds them.

## Measured facts

Qt 6.12.0 (`C:/Qt/6.12.0/msvc2022_64`), run offscreen. Probes are in the session scratchpad, not the repo.

- Built-in receivers (`qmltestrunner`, `scratchpad\p1\tp1.qml` and `tp2.qml`):
  - A typed `int` function parameter, and a built-in C++ `int` parameter (`ListModel.get`), take `undefined` and `null` as 0. `ListModel.get(undefined)` reaches row 0.
  - A typed `int` QML **property** does not: assigning `undefined` or `null` throws `Cannot assign [undefined] to int`.
  - A `var` property keeps `undefined` and `null`. A `variant` property defaults to `undefined`, and an explicit `null` stays `null`.
- cxx-qt receivers (`scratchpad\qvprobe`, built against the repo's fork at rev `ff48a9b`):
  - A cxx-qt `i32` invokable takes `undefined` and `null` as 0 (`select_row(undefined)` reached Rust as 0).
  - A `QVariant` `qproperty` with `NOTIFY` compiles and round-trips: `5` reads back as `5`, `null` as `null`, and `undefined` as `undefined`.
  - A JS `5` reaches Rust as `Some(5)` (`value::<i64>()`). A JS `null` reaches Rust as a null variant (`is_null()` true, type `std::nullptr_t`). A JS `undefined` also reaches Rust as a null variant (`is_null()` true, empty type name). `is_null()` alone can't tell them apart.
  - A `QVariant` property's default is `undefined` in QML.
  - Two properties sharing one custom `NOTIFY` compile. The writer sets both and emits once, and the handler sees the new pair.

## Proposed design

1. **File ids cross as decimal strings, with `""` for none**, as `BOUNDED_MEMORY_PLAN.md` D29 decides. The bridge already passes file ids as `i64` (`bridges/gallery_bridge.rs:187, 192`), so those sites change to the D29 form. `QVariant` works (measured), but it would add a second id convention beside D29, so it is not proposed for file ids.
2. **Action and selection codes become Rust enums with named variants** (`ViewerAction`, `PanelAction`, `RowKey`, `LedgerKind`, and the lane and notice codes). QML compares the names, not the numbers.
3. **Absent values are checked in Rust before a typed `int` receiver.** Typed `int` receivers take `undefined` and `null` as 0, and typed `int` properties throw. Neither is safe to rely on for "none".
4. **Counts and positions** follow item 5 of `TYPE_MISMATCH_VALIDATION.md` (`i64` in Rust, `real` in QML, and a window model). This plan does not decide them.
5. **The `QPersistentModelIndex` plan for item selections stays withdrawn.** A model-roles probe found that a Rust-owned selected role driven by intents works, and that no `-1` crosses the boundary. The replacement design belongs to Phase 7 step 7.9.

## Out of scope

- The saved last file (item 1 of `TYPE_MISMATCH_VALIDATION.md`). It crosses as a stamp in Rust, and the restore is handled in Rust (`crates/desktop/src/shell.rs:532`), so it needs no QML value.
- Counts and positions (item 4).

## Open

- Order against Phase 7 step 7.9, which decides the item selection design. The sweep for selections waits for it.
- Whether the action codes need the D29 string form. They are not ids, so the proposal is named enum variants, not strings.
