# Animated images that just play: the Yandex preview and grid, and the library's grid

## Context

The Yandex panel shows an animated result (GIF, APNG, animated WebP) as a still, in the grid and in the full-size preview. The
file is the real original, fetched from its host (`Hit.url`, kept as `Fetched.file.bytes`), and it is turned into one frame on
the way to the screen:

- The grid tile shows `Fetched.grid`, a JPEG made from the file (`yandex/images.rs`, `grid_picture`): one frame.
- `crates/desktop/src/providers.rs:99` (`yandex_picture`) decodes the original with `QImage::from_data`: one frame.
- `crates/desktop/qml/YandexPreview.qml:194` draws it with a plain `Image`: one frame.

Animated images should just play: in the preview, in the Yandex grid tiles (item 9) and in the library's grid (item 10).

The viewer animates these formats with `AnimationPlayer` (`desktop/src/animation.rs`) -> `pg_render::player` ->
`pg_codec::animation::frames` -> a `VideoOutput`. That path takes only a catalog file id (resolved to a path at
`animation.rs:115`, read by `std::fs::read` at `render/src/player.rs:86`). A Yandex result is in-memory bytes with no catalog
file.

**Standard: the most proper design, not the least churn.** No ambiguous states, no copies, no hedged parts, and the defects
found in the code this change rewrites are fixed with it (items 6, 7, 8). Churn across crates is accepted.

## Status

- Done: the three checks (below); item 1's frame count (`pg_metadata::frames`, `pg_render::player::animated`); item 2's
  `Bytes` type and `frames(Bytes)`; the raw EXIF column (item 8's storage): `analysis.exif_orientation`, written by the
  analysis (`analyze.rs`) and set exactly at each turn written into a file (`writeback` reads the tag from the bytes it wrote,
  `queue::write_done` stores it). Read into `Entry.exif_orientation`. No behaviour change yet: the viewer and the grid still
  turn by `orientation`.
- Also done: item 6 (`Player::drop` no longer joins the thread; a frame is presented only under the lock that `drop` takes to
  close, with tests for both); item 2's loader (`Player::start(load, sink, paused)`, `read_file`, and the viewer's catalog
  path lookup moved onto the player's thread); item 8 in the viewer (`PlayerTurns::of`: an animation is turned by its EXIF
  orientation, the catalog and pending turns, and mirrored when the orientation is; a video keeps the turns alone; the
  `TurnedVideoOutput` component turns the frames, with the mirror taken inverse to the turn because QML turns before it
  mirrors: `tst_turned_video_output.qml` checks the corners). QML: 156 tests pass; desktop: 93 pass.
- Yandex side, done: item 1 (`animated` and `orientation` on the fetched file, `Measured`, `Hit`); item 2 (`Fetched` is
  `AsRef<[u8]>`, `Images::kept` and `kept_image`, never waiting); item 3 (`ResultAnimationPlayer`, `ImageRequest::from_url`,
  the player reads the kept file on its own thread); item 4 (`previewAnimationUrl`, `previewMirrored`, `previewTurns`,
  set in that order); item 5 (`YandexPreview` shows the still under a turned picture, with the animation drawn over it once
  a frame is in); item 9 (`resize_rgba` premultiplied, the player's `Size::Height` scaling with tests, the tile's
  `ResultTile` loader that plays only while on screen, at rest, with the preview closed; roles `animationUrl`, `mirrored`,
  `turns`).
- Decision changed from item 7: the still is turned in QML (`YandexPreview`'s picture), not by a C++ shim on the decoded
  image. cxx-qt-lib 0.10's `QImage` has `flip` but no rotation, so a turn in C++ would need a new Qt binding. The decode
  stays Qt's, and the turn is the same one the animation gets (`TurnedVideoOutput`'s rule, mirror then turns, with the
  inverse turn when mirrored).
- Item 10, done: the viewer's `AnimationPlayer` takes a `height`; the roles `animated`, `playerTurns`, `playerMirrored`
  (`gallery/roles.rs`, `model.rs`, through `PlayerTurns::of`, the viewer's rule); `GridRow` copies them and gives each cell
  `itemPlays` (on screen with a row's margin, live, its pictures asked for, not pooled); `GridCell` plays its file over the
  still, at the row height, with a white backing once a frame is drawn; `GalleryGrid` gives the rows the scroll and
  `mayPlay` (viewer closed, settled, not moving).
- Gap, not closed: a modal panel over the grid does not stop the library's cells (the shell's panel state is in Rust, not a
  QML property). The fix is a QML property from the shell, not a change to the rule.
- Item 11: `docs/reference/REVERSE_SEARCH.md` has an "Animated results" section.
- Windowed run (2026-11-28), done on a scratch home with a synthetic session (`--yandex-fixture`: an animated GIF with
  transparency, an EXIF-6 still, an animated WebP): the preview's GIF animates (3.9% of its pixels change in 400 ms); the
  still shows upright and turned (its black corner at top-right, as a 90° clockwise turn gives); the Yandex panel's tiles
  animate (1.3% change, the webp's stripe moves). The run found one defect, fixed: the preview's video output kept the last
  frame of the previous animation and drew it over the next item's still; it is now hidden until its own frames show.
- Not done: the full `just lint`
  (clippy and machete; only the boundary check and `rustfmt` on the touched files ran), and a Rust unit test for
  `refresh_preview`'s URL and turn (the QML tests cover the preview's side).
- QML: 174 tests pass. Rust: pg-desktop 93, pg-render 19, pg-codec 34, pg-client, pg-app pass; the desktop build compiles
  the app's QML; the boundary check passes.
- The column is a baseline edit (no release yet, so no migration). A catalog carried over by a rebuild copies only the
  columns the old table had, so its `exif_orientation` is NULL until its files are re-analysed; reads fall back to the
  composed `orientation` (`coalesce`), which is what the old build used.

## Run-time checks that gate the design (done first)

Results (2026-11-27):

1. **Does `QImage::from_data` apply EXIF orientation to a JPEG? No.** A synthetic 4×2 JPEG tagged orientation 6 decodes as
   4×2 (`pg-desktop` test, Qt 6.12.1, offscreen). So item 7 stays: the preview's still is turned by the app.
2. **Does `VideoOutput` blend straight-alpha frames? Yes, the alpha is honoured.** Offscreen, a `VideoOutput` draws nothing at
   all (even an opaque frame stays black, under opengl, d3d11 and software), so the answer came from a windowed probe
   (`scratchpad/probe2`, Qt 6.12.1, the default Windows platform). A frame of 50%-alpha red (200,0,0,128) over black comes
   out as (143,0,0). A premultiplied read would give 200, and an ignored alpha would give 200; a plain sRGB straight blend
   gives about 100. So the frame's alpha is applied, and the transparent part shows what is behind it, which is item 5's
   need. The exact curve is not the sRGB one (143 rather than 100): nothing in the plan depends on it, and the end-to-end
   check of a transparent GIF over the preview's fill still applies.
3. **Does `analysis.orientation` already fold in `files.rotation`? Yes, but it goes stale.** Import composes the file's EXIF
   with the catalog turns (`codec/src/decode.rs:194`), so it includes them. After a rotation is written, the analysis is
   marked stale and kept with its old `rotation` until the re-import (`catalog/src/queue.rs:409`). A stored orientation
   therefore cannot be trusted between the rotation and the re-import.
   **Decision (you chose it):** item 8 uses a stored raw EXIF orientation, with no catalog turns, as a new column of the
   analysis (catalog schema change and migration). The turns then come from raw + `files.turns` + pending turns, always
   current.

Method: synthetic files made in the scratch area, a small test or a headless run with a trace; never a search of the machine
for samples.

## Design

### 1. "Animated" is a fact about the fetched file, found once, from the container (no decode)

Same rule as the catalog: more than one frame. `pg-render` already depends on `pg-metadata`; `pg-app` reaches both through
`pg-render`, so `tools/crate-layers.txt` does not change.

- `pg-metadata`: `pub fn frames(bytes, format) -> u32`, the container walk alone (as `pg_metadata::orientation` is the narrow
  read of the orientation), so a fetched result is not parsed for EXIF and XMP as `Meta::read` does. `Meta::read` takes its
  `frames` from the same walk: one rule.
- `pg-render`: `pub fn animated(bytes, format) -> bool` (`frames > 1`, in `player.rs`) and `display::orientation_of(bytes) ->
  Orientation` (sniff + `pg_metadata::orientation`, the reading the grid already uses).
- `pg-app` `yandex/images.rs`: `Picture` and `Fetched` gain `animated` and `orientation`, filled in `grid_picture` (the one
  `check` that already has the bytes). `Measured` gains `animated` and `orientation` (how the front end learns of a file coming in; its only
  consumer is `desktop/src/yandex.rs`, whose tests build one). `Hit` gains `animated: bool` and `orientation` beside `size`,
  set from `Measured`.
  `Fetched` implements `AsRef<[u8]>` (its file's bytes).
- `Images::kept(id) -> Option<Arc<Fetched>>`: the result's file if it is in, **never waiting**; exposed as
  `Yandex::kept_image(id)`. (`image()` / `get_known` wait on a fetch; the player's loader must not.)

### 2. Shared bytes end to end; the player's source is a loader

- `pg_codec::animation`: `pub struct Bytes(Arc<dyn AsRef<[u8]> + Send + Sync>)`, cheap to clone, itself `AsRef<[u8]>` (what the
  `Cursor` needs). Made by `Bytes::shared(Arc<T>)` for any `T: AsRef<[u8]> + Send + Sync + 'static` (the search's
  `Arc<Fetched>`: an unsizing coercion, no copy) and `From<Vec<u8>>` (a file read). Not from `Arc<[u8]>`: an unsized slice
  cannot become the trait object. `frames` takes `Bytes`; its other caller, `library/tests/video.rs:150`, changes with it.
- `pg_render::player::Player::start(load, sink, paused, size)` with `load: impl FnOnce() -> Result<Bytes, String> + Send + 'static`,
  run on the player's thread, and `size: Size` (`Native`, or `Height(px)`, item 9). `player::read_file(path)` is the catalog's
  loader (message "the file couldn't be read: {e}" kept). `play()` is otherwise unchanged (frame budget, loop, pause), except that
  a `Height` animation's frames are scaled before they are shown and before they are kept.

### 3. One playback core, two QML types (no mixed-source state)

`desktop/src/animation.rs`: a plain-Rust `Playback` holds what both need: the sink, the worker, `playing`, the error text, and a
generation counter incremented by each `start`. `Watched` (the player's sink) carries the generation and a
`report: Box<dyn Fn(u64, String) + Send + Sync>` that its owner builds from its own `qt_thread()`, so one `Watched` serves both
QObject types and a failure from a replaced run is dropped (replacing today's comparison of `file_id`, which is tied to
`AnimationPlayer`).

- `AnimationPlayer`: plays the catalog file `fileId`, and gains the same `height` property (0 = native size, the viewer's; the
  library's cells set it, item 10). It resolves the file's path in its loader on the player's thread, not on the window's thread in
  `restart()` as today (item 10 says why); an unknown file is a reported failure, as now.
- `ResultAnimationPlayer` (new QObject: `bridges/result_animation_bridge.rs` + state): `url`, `height` (0 = native size, the preview's; the
  tile's is its row height, item 9), `playerError`, `setSink` (it always plays: no `playing` property, and its `Player` starts
  unpaused). `url` is the preview's `image://yandex/<id>/original`; a URL, not a number, because result ids are
  `generation << 32 | index` and exceed what a QML number holds. `pg_client::image` gains `ImageRequest::from_url(&str)` (strip
  `image://`, `Provider::from_key`, then `parse`): the one reader of whole URLs, with its law in
  `crates/client/tests/image_laws.rs` (`from_url(request.url()) == Some(request)` for every request; no panic on any text).
  The player accepts only a Yandex original; anything else is a reported failure. Its loader calls `kept_image(id)`
  and hands the `Arc<Fetched>` to `Bytes::shared`; a result no longer kept (a new search, a dead result) is a failure the player
  reports.
- Keep the in-flight working-tree edits in `animation.rs` / `animation_bridge.rs` (`player_error_kind_of`, the `QMediaPlayer`
  include): they belong to another change; edit around them.

### 4. The preview learns "animated" with the item, as one value

`YandexSearch` (`desktop/src/yandex.rs`, `bridges/yandex_bridge.rs`, `bridges/yandex_state.rs`) gets `previewAnimationUrl`
(the original's URL when the item is animated, `""` otherwise) and, for item 7, `previewMirrored` and `previewTurns`. They are set
in `refresh_preview()` (which `apply_sizes` already re-runs when a file lands). One URL property, not a flag beside
`previewUrl`: properties are set one at a time and QML bindings run at each set, so "new URL, old flag" would start a player on
a still. The turn is set before the animation URL, so a player never starts with the last item's turn.
`YandexPanel.qml` passes `animationUrl`; "animated" in the preview is `animationUrl !== ""`.

### 5. `YandexPreview.qml`: the still is the poster, the player draws over it

The viewer's pattern (`ImageLayers` poster, `MediaView` player, the poster gives way on the first frame).

- One `Item { id: picture }` carries the geometry now on `full` (natural size from `full.sourceSize`, `fit`, `zoom`, `x`, `y`,
  `width`, `height`) and the `TapHandler`; `full` (still the source of `sourceSize`, `status`, `progress` and the error) fills it.
  `zoomable`, the 1:1 button, the Flickable's content size and `centre` run off `picture`.
- Over it, item 8's `TurnedVideoOutput` (turn and mirror from `previewTurns` / `previewMirrored`), shown after its first frame
  (`videoSink.onVideoFrameChanged`); from then `full.visible` is false. Reset on step, close and new item.
- `ResultAnimationPlayer { url: preview.animationUrl }`, sink set from the video output: an animation just plays, looping, from
  the moment its item is shown; it stops on close (the search clears the item, so the URL is `""`) and on a step to a still.
  No transport, pause button or key: the preview's keys (Space picks, a click or Z is 1:1, the arrows step) are unchanged.
- **Failure:** a non-empty `playerError` shows in the bar's third label with a new wording key in `ui.ftl` (the animation could not
  be played; the still is shown), never the English text. The wording key and golden checks follow.
- The hidden prev / next `Image` preloads stay: they warm the fetch and the first frame.

### 6. Stopping a player never waits for a decode (fix in `pg_render::player`)

Today `Player::drop` joins the player's thread on the window's thread, so a step or a close waits up to one frame's decode (a
hitch on a large GIF). The thread is the player's own; what must not happen is a frame reaching the sink after the stop.

- `play()` checks `stop` and calls `sink.present` in one critical section under the control lock (`wait_until` returns the
  guard); a frame is a hand-off of a copy, not a decode.
- `Drop` takes the lock, sets `stop`, notifies, releases it, calls `sink.close()`, and lets the thread end by itself (it holds
  its own `Arc` of the sink and the bytes). When `drop` returns no frame is presented and `close` has run. `Sink`'s doc states
  it: `close` once, from the stop, and no `present` after it.
- Both the viewer's animations and the preview gain it.

### 7. The preview's stills are upright, as the grid and `sizeText` already are (if check 1 says they are not)

`yandex_picture` applies no EXIF orientation, while the grid picture (`Maker::make_sized`) is upright and `sizeText` is the
upright size: a JPEG with an EXIF turn would show upright in the grid and turned in the preview.

- Not the app's own decode: the viewer's is opaque `Rgb` (`Rgba::opaque`), which would drop the alpha of a transparent
  PNG / WebP / GIF still. The decode stays Qt's and the orientation is applied to its result.
- A C++ shim in `cpp/providers.cpp` (cxx-qt-lib's `QImage` has no `mirrored` / `transformed`): `pg::turned(QImage, mirrored,
  quarter_turns)` using `Orientation::parts()` (mirror left-right, then turns clockwise), exact for multiples of 90 degrees,
  alpha kept. `yandex_picture` applies it to the original, from `Fetched.orientation`.
- The animation is turned the same way (item 8's component), or the poster and the frames would disagree, and the geometry taken from the
  upright poster would stretch the frames.
- Test: a synthetic JPEG for each of the eight orientations comes out the size `make_sized` reports and with the same corner
  pixel as the grid picture.

### 8. Animations are turned by the file's orientation in the viewer too (if check 3 allows)

`gallery/items.rs` `player_turns` turns a catalog animation by `turns + pending_turns` only: never the file's EXIF orientation,
and `playerTurns` is quarter turns, so the four mirrored orientations (2, 4, 5, 7) cannot be shown at all, while the grid
thumbnail and the stills are upright. The preview needs the same rule, so it gets one home.

- Derive turns and mirror from the orientation's `parts()` plus the pending turns, per check 3, so the catalog rotation is not
  applied twice. **Animations only** (`MediaType::Animated`): a video's own rotation is applied by Qt's player, so a video keeps
  `turns + pending_turns` and no mirror, or it would turn twice.
- New `viewedPlayerMirrored` beside `viewedPlayerTurns` (`gallery_state.rs`, `gallery_bridge.rs`, `model.rs`, `items.rs`, the
  `GalleryModel.qml` stub).
- One QML component, `TurnedVideoOutput.qml` (the box `MediaView` builds inline: `swapsAxes`, `rotation`, plus a mirror), used by
  `MediaView` and `YandexPreview`; `Viewer.qml` passes the mirror as it passes the turns. The mirror is a `Scale { xScale: -1 }`
  in `transform`; its order against `rotation` is not assumed: the component's test uses an asymmetric marker and checks all
  eight orientations' on-screen positions, and if QML applies them the other way the component uses the equivalent turn.

### 9. The Yandex grid tiles play too: only the ones on screen, at tile size (the rules item 10 reuses)

A tile plays its result's animation, from the same original the preview plays. Playing every result's animation at full size is
not an option (40 tiles of a 128 MB-frame GIF each), so:

- **Only tiles on screen play.** The panel passes its scroll range (`contentY`, the viewport's height, and each group block's
  offset in `sections`) and a tile plays when its frame intersects that range (with a margin of one row), the panel is shown and
  the preview is not open. A tile that scrolls away stops (item 6 is what makes starting and stopping players per scroll safe: no
  join on the window's thread) and shows its still again. What runs is bounded by the viewport, not by the result count.
- **At tile size.** `Player::start` takes `Size::Height(px)`: each frame is scaled to the grid picture's height (the panel's row
  height, as `grid_picture` makes it) before it is presented and before it is kept for the loops. The 128 MB frame budget is then
  almost never reached (a 220 px frame is small), so a looping tile decodes once and loops from memory; a tile's animation is as
  sharp as its still. `pg_codec::resize` gets `Resizer::to_height_rgba`: the same Magic Kernel Sharp 2021 as every other resize
  ("one resize for every pipeline"), on straight-alpha RGBA made safe by premultiplying first and unpremultiplying after (the
  kernel on `fast_image_resize`'s `U8x4` with its `MulDiv`), so a transparent edge does not fringe. Tests: an opaque frame comes
  out as the `Rgb` resize of it (the same pixels), a transparent edge keeps no dark halo, the size is the ratio's.
- **Starts at rest.** A tile starts its player once the view has stopped moving (the Yandex list's `moving` false; the library's
  `settling` false), and stops the moment it leaves the range, so a fast scroll starts nothing.
- **A white backing.** The grid pictures are flattened onto white (`decode.rs`: "transparency flattened onto white"), so a
  tile's frames are drawn over a white rectangle: the swap from still to animation does not change the picture. (The preview is
  different: its still keeps Qt's alpha, and its frames show over the preview's fill, as the still does.)
- **Crop as the still's.** The tile's still is `PreserveAspectCrop` into a tile that may be narrower than the image, so
  `TurnedVideoOutput` takes a `fillMode` (`PreserveAspectFit` for the viewer and the preview, `PreserveAspectCrop` for tiles); a
  centred crop commutes with the turn and the mirror, so it is applied inside the turned box.
- **Yandex roles.** The results model gets `animationUrl` (the original's URL when the result is animated, `""` otherwise),
  `mirrored` and `turns`: set when the file lands, with the size roles (`apply_sizes` already emits `dataChanged` for those; the
  new roles join its list). The tile's still is upright by `make_sized`, so its frames are turned and mirrored like the
  preview's (item 8's component).
- **`ResultTile.qml`.** Under the still `Image`, a `Loader` active while the tile plays: white backing, a
  `ResultAnimationPlayer { url: tile.animationUrl; height: <row height> }` and a `TurnedVideoOutput`, shown after the first frame,
  at which the still hides (as the preview's). The tile's click, double click, pick badge, size caption and keyboard focus are
  unchanged and stay above it.
- The preview and the grid never play the same animation at once (the preview covers the grid and stops it).

### 10. The library's grid plays its animated files too

The same machinery, on the library's `GalleryGrid` (a `ListView` of `GridRow` delegates, each with a `Repeater` of `GridCell`s,
pooled and reused, `cacheBuffer` 1500). Today an animated file shows its still and a badge (`ROLE_BADGES`, `GridCell.qml`).

- **What plays.** Only `MediaType::Animated` items (a video keeps its badge and still: it has no frame player, and plays in the
  viewer). `AnimationPlayer` plays the cell's catalog file `ids[index]`; its `height` is the thumbnail height the cell's still uses
  (`ThumbHeight`, the one `thumbUrls` is built with), so the animation is as sharp as the still and its frames small.
- **Which cells.** A cell plays when its row is on screen, not just instantiated: the row's `y` against the list's `contentY` and
  height, with a margin of one row (the `cacheBuffer` keeps rows well outside the view alive, and a pooled row has `loadStarted`
  false: that gate is part of the rule), the list is not `settling` and at rest, and the grid is the thing shown (no viewer, no modal
  panel: the state `GalleryGrid` and the shell already know). The badge stays on the cell either way.
- **Roles, per item, like `badges`.** `animated` (bool), `playerTurns` (int) and `playerMirrored` (bool), from item 8's rule
  (orientation + catalog turns + pending turns, animations only) through the same Rust function the viewer uses
  (`player_turns`, which returns turns and mirror): `ROLE_ANIMATED`, `ROLE_PLAYER_TURNS`, `ROLE_PLAYER_MIRRORED` in
  `gallery/roles.rs`, in `VALUE_ROLES` before `ROLE_FIRST` and in `ROLE_NAMES`, built in `model.rs` beside `ROLE_BADGES`. A turn the user
  makes changes the roles with the thumbnail URL, so the animation turns as the still does. `GridRow` gets the three lists
  (`required property`, `shown*`, the `onXChanged: take()` handlers), `GridCell` gets `itemAnimated`, `itemPlayerTurns`,
  `itemPlayerMirrored`; the stubs and `tst_gridrow.qml` / `tst_gridcell_*.qml` take them.
- **`GridCell.qml`.** The same `Loader` as `ResultTile`'s (white backing, `AnimationPlayer`, `TurnedVideoOutput` in crop mode),
  placed above the `thumb` and below the failed mark, the borders overlay (which is placed as the still is cropped, and stays
  right), the selection frame, the rating and the badge. The still keeps its fade: `thumb.visible` follows the first frame, its
  `opacity` and `shown` logic untouched (`tst_gridcell_fade.qml` reads `opacity`).
- **`AnimationPlayer` resolves the catalog path in its loader, on the player's thread**, not on the window's thread in `restart()`:
  a screenful of cells must not each run a catalog query on the window's thread. An unknown file is a reported failure, as now. A
  cell does not show a failure: the still stays, and the failure is logged once per file (`pg_log::warn`, as `restart()` does for
  the path error today); the viewer keeps the HUD text.
- **Measured, not capped.** A thread per playing cell, a screenful at most, each decoding once and looping from small frames.
  If many animated cells starting at once hitches the window in the end-to-end run, the decode moves to a shared pool of workers
  sized to the cores (a scheduler, not a memory cap); it is not built before it is seen to be needed.

### 11. Docs

`docs/reference/REVERSE_SEARCH.md` (or the Yandex section of `crates/README.md`): the grids and the preview play animated results.
Fix the doc comments in `pg_client::image` and `providers.rs`. `docs/webclient/WEB_CLIENT_DESIGN.md`: the new
`ResultAnimationPlayer` is a device-class type like `AnimationPlayer` (a browser plays the original file natively). The `GridCell`
header ("the badge of an animated image") and `roles.rs` docs say the cell plays it.

## Order of work and the working tree

1. The three run-time checks.
2. Lowest layer up: codec `Bytes` + `frames` + `to_height_rgba`; metadata `frames`; render (`animated`, `orientation_of`, the loader
   player with `Size`, item 6); client (`from_url`); app (`Picture` / `Fetched` / `Measured` / `Hit`, `kept`); desktop (`Playback`,
   the two players, `YandexSearch` and its roles, the provider and its C++ turn, `player_turns`); QML (`TurnedVideoOutput`,
   `MediaView`, `Viewer`, `YandexPreview`, `ResultTile`, `YandexPanel`), stubs; docs. The preview (items 1 to 6, 8) lands and is
   checked before the tiles (item 9), which add scaling, roles and the on-screen rule on top of it, and the Yandex tiles before
   the library's grid (item 10), which reuses them and touches the busiest QML (`GridRow`, `GridCell`) and the grid's roles.
3. Edits are made with the Edit / Write tools, one file at a time (never a script that rewrites files). The checks
   (`cargo check` / `test`, `just lint`) wait until all the edits are in.
4. The working tree already holds another change in progress (27 modified files, among them `animation.rs`,
   `animation_bridge.rs`, `MediaView.qml`, `app.rs`, `lib.rs`, `build.rs`, `ui.ftl`): edit around it, never revert or reformat it.
   No branches and no commits unless asked: the work stays on `main`, uncommitted.

## Deliberately not done (separate features)

- Videos in the library's grid keep their still and badge (no frame player for them; they play in the viewer).
- No loop-count handling (the player loops forever, as the viewer's does).
- `FRAME_BUDGET` (128 MB of kept frames per playing animation) stays: the no-caps rule bans configured caps and budget knobs and
  allows fixed working-set constants; this is one, per animation, and does not grow with the library.

## Critical files

`crates/codec/src/{animation.rs,resize.rs}`; `crates/metadata/src/{lib.rs,container.rs}`; `crates/render/src/{player.rs,display.rs}`;
`crates/client/src/image.rs` + `crates/client/tests/image_laws.rs`; `crates/app/src/yandex/images.rs`,
`crates/app/src/yandex.rs`; `crates/desktop/src/{animation.rs,yandex.rs,providers.rs,bridges.rs,build.rs}`,
`bridges/{animation_bridge,animation_state,result_animation_*,yandex_bridge,yandex_state,gallery_state,gallery_bridge}.rs`,
`gallery/{items,model}.rs`; `crates/desktop/cpp/providers.cpp` (+ header); `crates/desktop/qml/{YandexPreview,YandexPanel,ResultTile,GridCell,GridRow,GalleryGrid,MediaView,Viewer,TurnedVideoOutput}.qml`;
`crates/desktop/src/gallery/roles.rs` (+ `model.rs`) for item 10's roles;
`crates/wording/locales/en/ui.ftl`; `crates/library/tests/video.rs`; QML stubs and tests under `crates/desktop/tests/qml`. Reused
as they are: `pg_metadata::orientation`, `desktop/src/frame_sink.rs` + `cpp/frame_sink.*`, `Orientation::parts`, the viewer's
poster-then-player pattern.

## Verification

Per my notes: native Windows (PowerShell, `QT_DIR` set), `QT_QPA_PLATFORM=offscreen` for QML, a scratch catalog (`PG_HOME` /
`PG_CATALOG_DIR` / `PG_PREFS`), never `D:/PhotoGallery`, synthetic fixtures only; checks batched after all edits.

1. `cargo check` and `cargo test -p pg-codec -p pg-metadata -p pg-render -p pg-client -p pg-app -p pg-desktop`:
   - player: the existing tests on `read_file`; play from memory with no file; a failing loader (nothing shown, `failed` called,
     sink closed); item 6 (no `present` after `drop`, a slow-presenting sink, `closed` when `drop` returns, `drop` returns while a
     loader that sleeps or a frame decode is running);
   - `frames` / `animated` on a GIF of 1 and 3 frames, an APNG, an animated and a still WebP;
   - `images.rs`: a fetched animated file is `animated` in `Measured` and `Fetched`; `kept` never waits;
   - `refresh_preview`: the animation URL and turn for animated and still items, beside `the_preview_follows_its_picture_through_removals`;
   - item 9: `to_height_rgba` (opaque equals the `Rgb` resize, no halo on a transparent edge, the size from the ratio); a `Height`
     player presents and keeps scaled frames (sizes, and the kept bytes are the scaled ones); the model's `animationUrl` /
     `mirrored` / `turns` roles change when a file lands, and are empty for a still;
   - `from_url` laws; item 7's orientation tests; `player_turns` for the eight
     orientations (item 8); the `Measured` field in `desktop/src/yandex.rs` tests.
2. `just lint` (boundaries, clippy, fmt, machete, wording checks).
3. QML: add `ResultAnimationPlayer` to the stubs' `qmldir`; `tst_media_view.qml` for a mirrored and a turned animation;
   `tst_yandex_preview.qml` (inputs stubbed): a still has no player URL; an animated one sets it; step and close clear it; the poster
   gives way on the first frame; `playerError` shows the words. `YandexPreview.qml` names
   `YandexPanel.PanelAction`, which pulls `YandexPanel.qml` into the test's compile: if the stubs cannot load that, move the enum
   to its own `PanelAction.qml` holder (as `PanelKind.qml` holds its kinds) and point the preview, the panel and the enum-order
   test at it, rather than stubbing the whole panel. `tst_result_tile.qml`: a still tile has no player; an animated tile on screen
   sets the player's URL and height; scrolled off screen, with the preview open, or with the panel hidden, it is cleared and the still
   shows; the still hides after the first frame; the pick badge and caption stay above the animation. Item 10: `tst_gridcell_*`
   and `tst_gridrow.qml` take the three new lists; a new `tst_gridcell_animation.qml`: a still cell has no player; an animated cell
   in a live, on-screen row with the list at rest sets the player's file and height; a pooled row, an off-screen row, a settling list
   or a hidden grid clears it; the thumbnail's fade test is unchanged; the failed mark, borders overlay, selection frame, rating and
   badge draw above the animation. Rust: the three roles for an animated and a still item, and for each of the eight orientations
   with pending turns.
4. Animation orientation: synthetic animated WebP / APNG for each EXIF orientation (extend `pg_codec::fixtures` if it has no
   orientation parameter): preview and viewer frames come out the same way up as the poster and the grid thumbnail.
5. End to end (ask before a windowed run): `gallery --yandex-fixture=<dir>` (`yandex/replay.rs`), whose `downloads.tsv` maps result
   URLs to local files. Fixture folder in the scratch area: an animated GIF with transparent pixels, a still, a one-frame GIF, an
   EXIF-6 JPEG. Run headless with `--find-similar=<synthetic photo>` on a scratch catalog. Open the preview on each: the animated one
   reaches the sink and the poster hides (trace); the one-frame GIF and the still never start a player;
   step between them quickly (no hitch); the JPEG is upright and its `sourceSize` matches `sizeText`; the transparent GIF shows the
   preview's fill, not black. In the grid: with more animated results than fit on screen, only the visible tiles play (the trace
   shows players starting as tiles scroll in and stopping as they leave, with no hitch while scrolling fast), and the preview opening
   stops them all. The library's grid: a scratch catalog holding synthetic animated GIF, APNG and WebP files (with transparency and
   an EXIF turn), a still and a video, imported by the app: the animated cells on screen play at rest, the transparent ones over
   white as their stills are, the turned one the same way up as its still, the video keeps its badge and still; rotate an animated
   item (R) and its cell turns with it; open the viewer and the grid's players stop; fill a screen with animated cells (many small
   GIFs) and watch for a hitch as they start, which decides whether the shared worker pool of item 10 is needed.
