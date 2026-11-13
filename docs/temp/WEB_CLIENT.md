# Web client

How a browser client for the gallery should be built. Claims about other projects were checked against their source at the commits pinned below, and claims that could not be checked were removed. The rest was checked against this repo on 2026-11-12. It covers the frontend, how thumbnails get past the browser's connection limit, how video plays, what the server must send, and how state is split between client and server.

| Project | Stars | Server / client | Commit read |
|---|---|---|---|
| Immich (+ `justified-layout`) | 118k | NestJS / SvelteKit, layout in Rust→WASM | `b7510e7` (`justified-layout` at `6d5d2df`) |
| PhotoPrism | 40k | Go / Vue | `fe94b42` |
| Ente (web) | 30k | Go / Next.js, logic in Rust→WASM | `54e4e209` |
| LibrePhotos | 8k | Django / React | `eefb8d5` |
| Photoview | 6.6k | Go + GraphQL / React | `4abdab7` |
| Nextcloud Memories (+ go-vod) | 3.9k | PHP / Vue, Go transcoder | `dbf9f4e1` (Memories; go-vod has no pin yet) |
| PiGallery2 | 2.3k | Node / Angular | `e92fe15` |
| HomeGallery | 1.2k | Node / React | `5b6232c` |
| Spacedrive | 39k | Rust / React | `26b57e1` |
| lap | 4.2k | Rust (Tauri) / Vue | `f40520f` |
| Photofield | — | Go / Vue + OpenLayers | `d7374a9` |
| Urocissa | 300 | Rust (Rocket) / Vue | `102025d` |
| SPIS | 212 | Rust (axum) / htmx | `ceedbb9` |

Candidates came from GitHub topic search by stars (`photo-gallery`, `photos`, `photo-management`,
`google-photos`, `photography`), awesome-selfhosted's Photo Galleries list and Google.

## Status

**Decided** (approved in review): the controller runs in every client; the server is the only data authority and the only
process that writes the catalog; the desktop talks to the server over loopback; mutations are versioned sets; items are
identified by `FileId` on the wire; listings are stamped with a generation and freeze membership and order. From the latest
review: the server is async (`axum` on `tokio`); the CLI's writes go through the server while it runs; access is by login,
not pairing; a cache miss queues the work and returns a placeholder; `write_back` is a server-wide setting that the owner can
change from any client, on by default; deletes and moves are in version 1, once quarantine exists; scan changes reach clients
live, and a phone can start scans and imports; the layout runs in a Web Worker; the phone target is Android (Chrome) for now;
Yandex is in version 1; a server-side folder browser can add library folders; the budgets are set by measurement.

**Target, gated:** one shared QML frontend in the browser and on the desktop. The gate is an HTTPS run of the spike on a
phone (see "Reusing the QML").

**Open:** the video side (work in progress, kept open by decision); the listing model (a client-held replica, or OFFSET paging; investigated, not decided); the owner decisions in `WEB_CLIENT_DESIGN.md`; the budgets, once measured. The area designs are in `WEB_CLIENT_DESIGN.md` and are not yet reconciled with each other.

## Verdict

- **Frontend: one shared frontend, the QML app, in the browser and on the desktop.** This is the goal. Whether it is
  reachable is the open gate (see "Reusing the QML"): a spike rendered the real grid in a browser, but video and the
  phone checks are not done.
- **Fallback if the gate fails: a separate browser frontend over the same protocol.** Its framework is not chosen. The
  protocol and the state model don't depend on that choice, so nothing on the server side waits on it.
- **The hard problems are the same in every frontend, and none is solved by the framework:**
  - the browser's maximum element height;
  - per-item reactive state;
  - thumbnails held up behind the 6-connection limit;
  - video the browser can't play.
- **What we can do differently,** because we already have every aspect ratio and versioned thumbnail keys:
  - exact geometry for the whole listing up front;
  - a scrollbar that is right before anything has loaded;
  - thumbnails sized for devicePixelRatio and cached `immutable`;
  - on-demand remux and HLS for MKV/AVI instead of transcoding ahead.
- **Reusing the QML: possible for the views, not yet for the app.** The `.qml` files run in a browser under Qt for
  WebAssembly (a spike rendered the real grid; see "Reusing the QML"). The bridge has to be reshaped: its data reads move
  behind the data layer (see State model), and its control state stays local. Video and the phone checks are open. Each
  piece is a rewrite, not a port.

## What the surveyed projects send, and what goes wrong

### Lists and layout

| Project | Up front | What breaks |
|---|---|---|
| Immich | per-month counts | heights guessed (`layout-support.svelte.ts:10-18`), corrected on load; a 70k-asset month froze the page for 10–20 s (#28861) |
| HomeGallery | the whole catalog | 200k items on Android, 400k on a 16 GB desktop (#134) |
| PiGallery2 | the whole folder | 3k-subfolder folders slow; pagination for folder listing is still requested (#125) |
| PhotoPrism, Photoview | pages of 156 / 200 | paged; #2892, a request for a full-length scrollbar, is closed as completed |

- **Wire formats:**
  - Immich sends each month as columnar JSON, built in Postgres with `array_agg` and `to_json`
    (`server/src/repositories/asset.repository.ts`).
  - Memories sends 15 JSON fields per photo (`Db/TimelineQueryDays.php:102-116`), batched as `GET /api/days/1,2,3`
    (`appinfo/routes.php:48`).
  - PiGallery2's short keys and string tables cut a listing to 11.9 MB from 33 MB, but gzip alone took it to 1.5 MB
    (#437). Compression comes first.
- **Reactive state is the real cost.** Immich's #28861 froze the page for 10–20 s with only 54 `<img>` on it. The issue
  attributes the freeze to layout pre-computation in `TimelineManager` and `TimelineMonth`. The code also keeps one
  reactive object per asset (`timeline-day.svelte.ts:172-174`). Immich #32179 (open) reports the timeline freezing
  after repeated small selections on a 313K-asset library. Positions and selection belong in typed arrays and id sets,
  outside any framework's reactive objects.
- **Urocissa's `get_prefetch.rs`** is the closest to our `list_grid(folder, recursive, Query, Sort)`. Its prefetch record
  carries a timestamp, a location and a length, so the client can fetch ranges of a fixed list.

### The maximum element height

Two projects have it open or worked around: Immich (#16788 open, #30061 and #31358 closed) and Memories (#1715 open).

Firefox caps an element near 17.2–17.9 M px (Memories #1715: 17,187,496 px; Immich #30061: about 17,895,624 px). #30061
reports the container clamped; #1715's reset to 0 is the reporter's hypothesis. The Chromium figure is not established,
so it is not used here. A million items in a justified grid at about 5 per 235 px row is about 47 M px. The scroll
position has to be our own coordinate, scaled onto a spacer below the cap, from the start.

### Thumbnails and the connection limit

`—` means not checked.

| Project | Sizes | URL / caching | Placeholder |
|---|---|---|---|
| Immich | 250 px WebP, 1440 px JPEG preview | `?c=<thumbhash>`, `private, max-age=86400` | thumbhash |
| PhotoPrism | 224/500 squares, fits to 7680 | SHA1 + size, 30 days immutable | — |
| Photoview | one 1024 px JPEG | 1 year immutable | blurhash |
| HomeGallery | 128–1920 JPEG, at import | immutable, 2 days (`packages/server/src/app.js:69`) | dominant colour |
| Memories | 256/512 | etag + index time | — |
| PiGallery2 | — | `max-age=86400` (1 day) | — |
| LibrePhotos | 250/500 high WebP | **no Cache-Control** | dominant colour |

- **The eight servers checked (all but Memories) serve no TLS or HTTP/2 themselves.** They live with 6 connections per
  host.
- **Mitigations in use:**
  - Immich aborts off-screen `<img>` fetches from a service worker (PR #16893, which cites the limit). That needs a
    secure context, so it does nothing on `http://<LAN-IP>`. Immich #27585: long-running video streams held all 6
    sockets, and thumbnails stalled (a comment on the issue).
  - LibrePhotos mounts full images only once scrolling slows, with a 2500 px buffer ahead and 100 px behind.
  - Spacedrive multiplexes every event over one shared SSE stream, because one stream per subscription "exhausts
    the pool and stalls RPC POSTs" (`packages/ts-client/src/transport.ts:236-240`).
- **Memories' `multipreview`** (`lib/Controller/ImageController.php:139`) batches thumbnail requests. Our batch design
  below is modelled on it.
- **Costs to avoid:**
  - PiGallery2's first open of a folder of over 10,000 pictures took almost 3 minutes (#613).
  - Spacedrive re-encodes a PNG per request and sends `no-cache` on a versioned URL
    (`apps/server/src/main.rs:228-279`).
  - Thumbnails must be served from the cache alone, never touching E:.

### Viewer

- **Zoom stops short:** at the 1440 px preview (Immich), or at HomeGallery's default preview size of 1920 px
  (`packages/extractor/src/extract/image/Image.schema.json`).

### Video

- **Immich:** real-time HLS is alpha and off by default (`server/src/dtos/config.dto.ts:130`), with 2 s segments
  (`constants.ts:245`), prewarmed variants, and hls.js in the web client.
- **Memories (go-vod):** 3 s segments (`main.go:19`). H.264 at max quality is served unchanged
  (`gv_stream.go:186`); other sources are transcoded in chunks (`manager.go`).
- **Transcoding ahead doesn't fit 21 TB on an HDD.** HomeGallery's video preview generation is too slow on a Raspberry
  Pi (#145).
- **go-vod's model fits us:** serve what the browser can play unchanged, and transcode the rest on demand. Its known
  problem is two ffmpeg processes starting at playback start (`pulsejet/memories#1560`), so our pipeline needs exactly
  one per playback.
- **Grid video tiles must release their player:** pause, clear `src`, call `load()` (LibrePhotos
  `Tile.tsx:47-55`). Without that the browser runs out of media players (LibrePhotos #2026).

### Realtime and auth

- **Live updates:**
  - Immich's socket.io updates are applied every 2.5 s (`websocket-support.svelte.ts`).
  - HomeGallery uses SSE (`packages/server/src/api/events/index.js:68`), which on HTTP/1.1 permanently takes one of the
    6 sockets.
- **Auth:** an httpOnly SameSite=Lax cookie lets `<img>` and `<video>` load with no token (Immich, LibrePhotos). A token
  in the path makes URLs unique per session and defeats caching (PhotoPrism).

### Rust→WASM in other projects

- **Ente's Rust→WASM:** crypto and similar logic, built with wasm-bindgen and tsify via `wasm-pack --target bundler`
  (`rust/bindings/wasm/lib/Cargo.toml`). The build constraints are:
  - webpack, not Turbopack (`web/packages/base/next.config.base.js:120`);
  - a lint check on WASM imports (`web/checks/wasm-imports/`), so WASM comes through `import()`;
  - a pinned nightly Rust for the cast binding (`rust/bindings/wasm/cast/rust-toolchain.toml`).
- **Immich's `justified-layout`:** about 200 lines (207 in `src/lib.rs`), `no_std` on wasm32, inlined as base64, with
  `initSync`. It takes a `Float32Array` of ratios and returns a `Float32Array` of positions.
- **Spacedrive's type generation** writes `packages/ts-client/src/generated/types.ts` (6,375 lines) with a method map,
  committed to the repo. That is how a TypeScript client gets its types. We don't need it for a Rust client, since the
  protocol types are Rust on both ends. Spacedrive's default branch no longer uses rspc; its server runs axum
  (`apps/server/src/main.rs`).

### Photofield: the grid as map tiles

Photofield renders the whole layout on the server as 512 px map tiles (`README.md:113-115`).

- **Gains:** requests scale with the viewport, not with the photo count, and opening a photo is a zoom into the same map.
- **Costs:** with no per-item DOM, hover, selection and badges need round-trips. The README says it is "not optimized
  for many clients". It panics on huge scenes (#54) and has memory issues (#193).

It's a poor fit for rating and selecting.

## State model

Decided. The controller runs in every client. The server is the only data authority, and the only process that writes the
catalog. The desktop talks to a per-user background server on loopback, through the same protocol as every other client.
Everything else in this section is *Proposed* unless it says otherwise; the open items are in "Open points".

### Ownership

- Server: catalog facts (items, ratings, marks, rotation, folders, the tree), scan and job status, persisted
  preferences, thumbnails, listing snapshots, and the event log.
- Client controller: the current item, selection, panels, viewer, grid size, scroll, filter-bar UI state, and timers
  (time is an input, `now_ms`).
- Preferences: per user, `show_borders`, the last file, the session listing and saved views (they follow you to a phone).
  Per device, `row_height`, `dev_mode` and `yandex_folder`. `real_size` is removed: photos always open fitted, and the 1:1 button stays for when a user wants real size. `write_back` is a server-wide setting, changed
  from any client by the owner, on by default.
- *Proposed:* a device record, created at login. The desktop is a device too, with a per-start loopback token. Device
  identity is open (below): the record's key, and the snapshot cap that counts per client, depend on it.

### Identity

- Items are identified by `FileId` on the wire. An index is local to one snapshot on one client and never crosses the
  boundary. The client's data layer converts index-carrying commands (`Select(usize)`, `Rate { item }`,
  `ShowViewer(usize)`) to `FileId` using its own snapshot.
- Every message that refers to a listing carries `(listing_id, generation)`. The server rejects a mismatch with a typed
  error, so a stale index can't reach the wrong file.
- The `Event` and `Command` enums in `crates/client/src/controller.rs` use indices (`Select(usize)`, `Rate { item }`). They
  stay local. The wire message set is defined from this section, not from those enums.

### Listings

- **What a snapshot is:** when a listing opens, the server runs the query and sort once, and keeps the resulting ordered list
  of item IDs under a listing ID and a generation. It stays fixed until the client closes it, it expires, or the client reopens
  it. The server holds about 56 bytes per entry in memory (`docs/temp/HEAP_PROFILE_START.md:63`), so roughly 56 MB per million
  items. The client gets the aspect-ratio block once, then fetches item records by range as the grid asks for them.
- A snapshot freezes membership and order when it opens. Attribute changes update items in place.
- The server publishes the fields each listing's query and sort read. A change to one of those fields is
  membership-affecting. The client then reopens the listing with the same query and sort, keeps the selected `FileId`,
  and anchors the scroll to it. If the selected item no longer matches, the selection moves to the next item in the new
  order, or the previous one if there is none.
- *Proposed:* a snapshot is released after its client disconnects, after a grace period long enough for a sleeping phone
  (the length is open). A 10-minute idle backstop applies, and each client may hold at most four snapshots. Every number is
  unmeasured. The cap needs a client key, which depends on device identity.
- *Decided, version 1 membership changes:* scan-missing files, library folder removal, and rating-driven sort. The desktop
  edits a rating in place today, so for the web client it is a membership rule, not a new behaviour. Scan-driven changes
  reach clients live.
- Using an expired snapshot returns `SnapshotExpired`, and the client reopens it as above.

### Mutations

- Every mutation is a set: `set(field, value, expected_version)`. Relative operations (rotate by one turn) are computed by
  the client into absolute values before they are sent.
- The server applies the set only when the item's version matches, and returns the new version.
- A client keeps at most one mutation in flight per item and queues later ones behind the returned version. Key repeat
  therefore serialises instead of conflicting.
- A version mismatch means another device changed the item. The client refetches the item, shows a notice, and doesn't
  retry silently.
- There is no offline mutation queue in version 1. A mutation with no server fails visibly.
- *Open:* the set-only rule has to be checked against the desktop's per-field edits in `crates/desktop/src/gallery/edits.rs`
  before it is final.

### Sync

- The server numbers events per session. The client records the last number it applied.
- On a gap or a reconnect the client calls `resume(seq)`. If the server still holds those events, they replay. Otherwise the
  client refetches the tree, the listing it shows and the item records it holds, then continues from the new number.
- The mapping from server changes to controller events lives in one place, the client's data layer. Its tests are part of
  the design.

### Loading

- Phases: `Connected` (protocol version checked), `TreeLoaded`, `ListLoading(generation)`, `ListReady(generation)`,
  `ListShown`.
- `ListReady` means the ratio blob is complete. Navigation (`Step`) and layout depend only on it. Item records arrive by
  range, and a grid cell without its record shows a placeholder.
- Commands that need a ready listing are ignored until `ListReady` for the current generation. No command reads a partly
  loaded listing silently.
- `ListReady` matches the reader's `Session::grid` completion (`crates/app/src/reader.rs:257`), not the window being ready.

### Protocol and versions

- The handshake carries a protocol version and a build id. The desktop refuses a server with a different protocol version,
  and restarts its own background server. A browser checks the build id on connect and reloads if the server has moved on.
- The server serves the frontend it was built with, so the browser and the server always match.

### Processes and access

- One binary, `gallery`, with the `serve` subcommand. An always-running per-user background process, started at logon and
  supervised by the desktop. The desktop attaches to it, or starts it, under a single-instance lock.
- Scans and sync run in the background at low I/O priority, and yield to the desktop. A phone can start scans and imports.
- The loopback listener binds `127.0.0.1` only. Requests carry a per-start token from a user-only file, so other local
  processes can't use it.
- The LAN listener is off by default. Once it is on: TLS with a certificate that matches the address the phone uses (the
  private CA certificate with the LAN name and the IP, already configured; the phone trust test is the gate), login before
  any request other than login, the `Origin` check on the WebSocket, and a logged-in credential on every mutation. The
  enforcement is in the transport, not in the client.
- *Proposed safeguards:* a connection cap, a pre-auth idle timeout, and rate limits on the login request. No values are set.
- **Login (decided, replaces pairing):** the owner sets a password on the desktop at first run. The phone logs in with it and
  receives an httpOnly cookie bound to a device record the desktop can revoke. Login needs no desktop window, so it works while
  the desktop is closed. *Open:* how the password is stored and hashed, lockout and rate limits, the reset path, and no QR code
  (decided).
- **CLI writes (decided):** while the server runs, the CLI's writing commands (`gallery scan`, `gallery import`) go through the
  server's API. Otherwise they write the catalog directly. No refusal is needed: ordinary SQLite writes already share the
  catalog. The exclusive hold (`crates/catalog/src/lifecycle.rs:49-52`) is taken only for layout changes.

### Testing

- Controller logic: the existing replay tests.
- Data layer: a conformance suite against the server, covering versions, sequence numbers and resume, and snapshot
  generation mismatch.
- Integration: the desktop over loopback, and one phone session per release.
- Budgets (decided: measure first): record the in-process baseline for listing open (`Session::grid` completion) and for the
  range fetch, then set the budgets from those numbers. The candidate numbers, unmeasured: listing open no more than the
  in-process time plus 50 ms; a 100-item range fetch with p95 at most 16 ms on loopback.

### Open

- The video side (work in progress; kept open).
- The TLS library for the LAN listener: `rustls` or `native-tls` (see Server).
- The login details (see Processes and access).
- The snapshot grace length for a sleeping phone. The 10-minute backstop and the cap of four are proposals.
- Device identity: the record's key, and the client key for the snapshot cap.
- The controller's view state. A serialisable struct has to be split out first, as a refactor prerequisite.
- *Proposed:* the desktop cuts over before the browser frontend, in this order: in-process timings, the server and its
  conformance suite, the benchmark, then the cutover. *Open:* whether the desktop's writes (edits, scans, thumbnails,
  write-back) move in the same step as its reads. If they don't, two processes write, which the sole-writer rule rules out.

## Design for us

**Server** (a second front end on a UI-free service crate):

- **Listing snapshot:**
  - `list_grid(folder, recursive, Query, Sort)` frozen under an id (the closest precedent is Urocissa's `get_prefetch.rs`).
  - Comes with one binary blob of every item's aspect ratio (2–4 B each, about 2–4 MB per million) and its
    thumbhash, both column-wise.
  - Everything else (path, rating, flags, details) by index range.
  - Compressed (PiGallery2 #437).
- **Thumbnails:** served from the cache only, at `…/<file_id>@<mtime>.<rotation>/<height>`,
  `Cache-Control: public, max-age=31536000, immutable`. A height the cache lacks gets the nearest
  (`ThumbCache::get_nearest`).
- **Batching:** a `multipreview`-style stream, with requests in viewport-priority order and cancellation.
  - Carried on the one WebSocket, which also carries events, so plain HTTP on the LAN never runs out of sockets.
  - The client stores each image in Cache Storage under its single URL.
  - With HTTP/2 over the LAN certificate, plain `<img>` would also do. The async server gives HTTP/2 natively, so no second
    process is needed. Which TLS library carries it is open (see Server). Nothing here is built yet.
- **Viewer:** a fitted image at screen × DPR, then zoomed regions; JPEG for formats the browser can't decode.
- **Video (work in progress; open):**
  - Original with byte ranges when the browser can play it (`canPlayType` on an exact codec string).
  - Stream-copied to fMP4 or HLS when only the container is wrong.
  - Real-time HLS transcode otherwise, paced in-process, with NVENC on the 2060.
  - Nothing ahead of time.
- **Events and auth:** see State model. Auth is an httpOnly cookie set at login.

**Frontend** (the shared QML app, see "Reusing the QML" for the gate):

- **Shape:** the existing `pg.gallery` views. The bridge is reshaped to the State model: control members read the local
  controller, and data members read the data layer. In the browser the data layer is the protocol; on the desktop it is
  the loopback connection to the same server.
- **Layout:** `layout::justify_into` (`crates/client/src/layout.rs`; the greedy core is about 20 lines, std plus
  `pg_model::num::count`), in `pg-client`'s WASM, run in the frontend's process over the whole listing. It reruns on
  resize, so the grid matches the desktop exactly. It runs in a Web Worker (decided); the cross-origin isolation from the HTTPS plan makes that possible.
- **Scrolling:** our own scroll coordinate scaled under the element-height cap, rows windowed by binary search and images
  mounted only once scrolling slows (LibrePhotos).
- **State:** positions in typed arrays, selection as id sets and index ranges, select-all on the server. No per-item
  reactive objects.
- **Logic:** `pg-client` is wasm32-clean (`clippy.toml` bans clocks, threads and files; `just lint` runs its wasm32 check,
  `tools/lint.py:21`). It runs in every client as WASM, controller included (see State model). Nothing is built yet. The
  replay format (`PG_RECORD` writes one `Update { now_ms, event, commands }` line per event) tests the controller in each
  client against the desktop's traces.
- **Viewer:** the existing `Viewer.qml`, fed fitted-image URLs, with zoomed regions from the view endpoint. Video in the
  browser is an open check (see Open checks). The current code preloads up to eight neighbours
  (`Viewer.qml:396-404`, `keys_around(item, 4)` in `crates/desktop/src/gallery/model.rs:335`), so the decode budget must count
  eight, not two.

## Server (draft)

The server is what every client talks to. It owns the catalog, the thumbnail cache, the jobs and the file access.
It has no window and no Qt. This section is a draft: the open decisions are at the end.

**What the repo has to build on:**

- `pg-app` is the typed API over the catalog, thumbnails, jobs, library and Yandex (`crates/app/src`, 114 public
  functions in 16 files; `library.rs` has 21). The desktop window reaches data only through it, so the server can too.
- `pg-model` has the `Query`, `Sort` and `Listing` types, and `pg-client` has the `Event`/`Command` enums. Both ends of
  the protocol are Rust with serde, so the protocol needs no generated types. `specta::Type` is derived behind a feature
  in `pg-model`, and it is needed only for a non-Rust client (the fallback).
- `crates/gallery` builds without Qt when its `desktop` feature is off (`crates/gallery/Cargo.toml`). A server can be a
  `serve` subcommand behind a `web` feature, next to the window.
- The repo has no async runtime and no async code. 24 files spawn threads, and `pg-app` is blocking (rusqlite,
  crossbeam channels). `Cargo.lock` has no HTTP server or WebSocket crate (no `axum`, `hyper`, `tiny_http` or
  `tungstenite`). Its one HTTP crate is the `ureq` client that `crates/yandex` uses.

**Crate:** a new `crates/web` (`pg-web`), with no Qt dependency. It depends on `pg-app` and `pg-client`, and holds the
HTTP and WebSocket server, the routes, the cache headers and the auth. `crates/gallery` gets the `serve` subcommand.
The built frontend (the QML WASM bundle, or the fallback's bundle) is served from a directory first, and embedded later.

**Stack (decided: async).** `axum` on `tokio`, with HTTP/2 and WebSockets from `hyper`. It is the project's first async code,
and every `pg-app` call goes through a blocking pool sized to the catalog's connections. The sync option (`tiny_http` with
`tungstenite`) is rejected: it has no HTTP/2, and a thread per connection does not suit a WebSocket per client with streamed
thumbnails. `REFACTOR_PLAN.md:136` already says "the axum server later", which agrees with this.

**TLS library (open):** `rustls` or `native-tls`. The choice matters for HTTP/2. The Windows backend of `native-tls` 0.2.18 has
no server-side ALPN (`native-tls-0.2.18/src/imp/schannel.rs` has no server accept path), so it cannot negotiate HTTP/2 on
Windows. `rustls` is pure Rust and supports server-side ALPN. It does not read the Windows certificate store by default, which
the LAN setup does not need, because the server loads its own certificate from a file. The repo's only current TLS user is
`ureq`, through `native-tls` (`Cargo.lock:1434`). Choosing `rustls` adds a second TLS stack beside it.

The LAN listener needs a connection cap, and thumbnail reads need a handoff from the async runtime to the blocking pool.

**Endpoints (sketch):**

- `POST /api/listings`: a `Query` and `Sort`. Freezes the result under an id and returns the count.
- `GET /api/listings/{id}/ratios`: the aspect ratios as one binary column, 2 to 4 bytes each.
- `GET /api/listings/{id}/items?from=&to=`: the rest of each item as columnar JSON, gzip-compressed.
- `GET /api/thumbs/{file_id}@{mtime}.{rotation}/{height}`: cache only, `Cache-Control: public, max-age=31536000,
  immutable`. A missing height gets the nearest one (`ThumbCache::get_nearest`).
- `GET /api/files/{id}/view?w=&h=`: a fitted image, or the original bytes with `Range` for video.
- `POST /api/login`: login, which sets the httpOnly cookie.
- `POST /api/items/{file_id}/set`: `field`, `value`, `expected_version`. Returns the new version, or `VersionMismatch`.
- `POST /api/resume`: `seq`. Replays the missed events, or returns `ResyncRequired`.
- Listing endpoints carry the generation (see State model).
- `GET /ws` (WebSocket): events as JSON, and the thumbnail batch as binary frames with request ids, in viewport order,
  with cancellation.

**Behaviour the server must have:**

- **Snapshots expire.** Expiry and recovery are defined in State model (`SnapshotExpired`, then reopen by query).
- **The cache is the only source for thumbnails.** A miss is queued, never read from the originals on the request path.
  Decided: the server queues the work and returns a placeholder. The current code reads the original on a miss
  (`crates/app/src/images.rs:179-191`); that path changes with the server.
- **Auth.** The WebSocket checks the `Origin` header, because the cookie is sent with it and cross-site pages can open
  it otherwise. The server binds to the LAN address only when asked.

**Open decisions:**

- **Stack:** decided, async (see Stack). The TLS library is open.
- **Catalog writers:** decided. While the server runs it is the only process that writes the catalog, and the CLI's writes go
  through it (see Processes and access).
- **Login:** decided over pairing. The details are open (see Processes and access).
- **Frontend gate:** see "Reusing the QML". It decides whether the fallback frontend is needed.
- **Thumbnail miss:** decided: queue the work and return a placeholder.

## Reusing the QML

**What the QML is made of** (`crates/desktop`):

- **Views:** 34 QML files, 5,115 lines, in the `pg.gallery` module. `Theme` and `Ratings` are singletons.
- **Bridge:** 19 Rust files each register one type the QML uses, with `#[qml_element]`. They declare 272 members: 112
  properties, 140 invokables and 20 signals. The largest are `shell_bridge.rs` (56 attribute lines), `gallery_bridge.rs`
  (36), `yandex_bridge.rs` (26) and `viewport_bridge.rs` (23). Each member reads in-process Rust state: the catalog, the
  thumbnail cache, the job queue. Under the State model, control members read the local controller and data members
  read the data layer, so only the data members change where they read from.
- **Qt modules:** `Quick`, `Controls.Fusion`, `Layouts`, `Window`, `Multimedia` (`MediaView.qml`: `MediaPlayer`,
  `VideoOutput`, and animation frames through the same `VideoOutput`), and `QtQml`. There is no `Dialogs` module; the
  folder pickers are `FolderPickerDialog.qml` and the system's dialog (see Files).
- **Native glue with no browser equivalent:** `cpp/providers.cpp` (a `QQuickAsyncImageProvider` over the Rust decoders),
  `cpp/frame_sink.cpp` (Rust threads feeding a `QVideoSink`), `cpp/gpu_device.cpp` (the app's D3D11 device, shared
  with Qt Quick and the player), and the screenshot and live-reload helpers. The browser build needs a replacement for each.

**What is already in place:** `crates/desktop/tests/qml/stubs/pg/gallery/` has stand-ins for 9 of the 19 bridge types
(160 lines of QML), and `tools/qmltest.py` runs the real views against them. So the views depend on names, not on the
Rust implementations. That is the seam a browser backend would plug into. It covers 9 types, though, and the stubs hold
only what the tests read. A full `pg.gallery` for the browser has to implement all of the surface.

**What blocks it:**

- **Build target.** The shared crates build for `wasm32-unknown-unknown` (`tools/lint.py:21`, run by `just lint` in CI;
  `rust-toolchain.toml` adds the target). Qt for WebAssembly and CXX-Qt need `wasm32-unknown-emscripten`, with POSIX
  threads and an Emscripten version that matches the Qt build. The
  [CXX-Qt WebAssembly guide](https://kdab.github.io/cxx-qt/book/concepts/wasm-builds.html) gives the version pairings.
  This repo has no such build. The desktop crate uses `cxx-qt` 0.10 (`crates/desktop/Cargo.toml`). The local Qt 6.12.0
  install has `wasm_multithread` and `wasm_singlethread` kits.
- **Threads and secure context.** Qt's threaded build needs `SharedArrayBuffer`, which a browser only enables on a
  cross-origin isolated page. Qt's own WASM test server sends the isolation headers, with the comment "required for
  WebAssembly threads" (`qtbase/util/wasm/qtwasmserver/qtwasmserver.py:67-68, 213`). Isolation needs a secure context.
  Over plain `http://<LAN-IP>` only the single-threaded kit would run, and it has no Multimedia (`wasm_singlethread` has
  no `Qt6Multimedia` CMake config). *Configured:* HTTPS with a private CA whose certificate has the LAN name and the IP address, installed on the phone. That makes
  the page a secure context, so the multithreaded kit can run. The WebSocket must be `wss://`, since a non-local `ws://` from an HTTPS
  page is rejected (`qwebsocket_wasm_p.cpp`, about 148–172). Phone trust is untested.
- **Video.** Qt Multimedia on WASM plays through an HTML5 `<video>` element that it creates on the page
  (`qtmultimedia/src/plugins/multimedia/wasm/common/qwasmvideooutput.cpp:579`). A clip bundled as a `qrc` resource is
  copied to a `file:///tmp` URL, which the browser rejects, so the source has to come over HTTP. Over HTTP the element
  was removed during start-up and `play()` was aborted (`AbortError`). The spike ran with the isolation headers already
  set, so isolation is not the cause. The cause is open (see Open checks). `QWasmVideoOutput::stop()` removes the element unless
  `m_toBePaused` is set (`qwasmvideooutput.cpp:321-355`), and `qwasmmediaplayer.cpp:336-337` does not return early while the
  player is Preparing, so a source change in that state reaches the removal. The browser build also cannot play the
  desktop's device source (`crates/desktop/cpp/video_source.cpp:25-33, 84-92`); it needs a served URL, which changes a Rust
  contract. Video is the blocker for `MediaPlayer` in a browser.
- **Files.** The folder pickers are used three times: `Main.qml:158` and `Main.qml:175`, and `YandexPanel.qml:127`. On
  Windows the system's dialog opens; elsewhere `FolderPickerDialog.qml` does (`crates/desktop/src/picker.rs`). Browsers
  pick files their own way, and the library's folders live on the server, so these dialogs become a server-side folder
  browser. Not tested in a browser.
- **Size and start-up.** Measured on the spike: 27.7 MB of `.wasm` (9.6 MB gzipped), 294 KB of JS (67 KB gzipped), and a
  13 KB loader. Cold start on a phone is not measured.

**The spike.** One screen, built outside the repo with Qt 6.12.0 and Emscripten 5.0.5. It uses the real `GridRow` and
`GridCell`, with `Theme` and `Ratings` as singletons, over 400 generated rows of five cells, plus a `MediaPlayer` on a
short MP4. It ran in desktop Chromium, with the server sending the cross-origin isolation headers.

- **Build:** works with the multithreaded kit (`wasm_multithread`), the only one with Multimedia. A QML module build
  needs every file listed (`BorderOverlay.qml` included), and the singletons declared before `qt_add_qml_module`.
- **Layout:** the 400 rows lay out at the right geometry (150 px rows, widths from the aspect ratios). The console was
  clean.
- **Images:** none. The spike passes no thumbnail URLs, since the image providers are Rust code and not in the spike.
  The cells show only what `GridCell` draws: its overlay colours, including the `#ff4ad8` square for unmarked borders.
- **Video:** not playing. The status and error log lines were `player media status 6`, then `AbortError: The play()
  request was interrupted because the media was removed from the document`. A `qrc` URL as the source fails with
  `NotSupportedError`.
- **Not run:** the single-threaded kit (it has no Multimedia), the HTTPS check with a trusted LAN certificate, touch on a
  phone, and cold start.

**The gate.** Re-run the spike over HTTPS with the trusted LAN certificate, with the WebSockets module, on the phone's
browser. It has to show the grid, thumbnails and video. If video works, the frontend is the shared QML app. If video
can't be fixed, the fallback is a separate browser frontend over the same protocol; the server and the state model don't
change either way.

What this settles: the QML views run in a browser under Qt for WASM, so the `.qml` files are not the obstacle. What it
leaves open is video, the data layer (images and data come over the wire, not from Rust in-process), and the phone
checks.

## WebChannel over WebSockets

One way the desktop app could serve its bridge objects to a browser: Qt WebChannel over a `QWebSocketServer`. The
browser gets a proxy for each object: properties, signals and invokables. This is ruled out. The goal is a headless
web gallery, not a remote view of the desktop app, and WebChannel needs the app running. The facts come from the Qt
6.12.0 source (`C:\Qt\6.12.0\Src\qtwebchannel`, `qtwebsockets`).

**What Qt publishes.** Every property, with its notify signal. Every public `Q_INVOKABLE` method and public slot.
All signals and enums. Property changes are batched on a timer, 50 ms by default (`propertyUpdateInterval`, with
`blockUpdates` to pause). Arguments and results are JSON (`qmetaobjectpublisher.cpp`).

**What it needs that Qt does not give:**

- A WebSocket transport. The library has none; `examples/webchannel/standalone/websockettransport.cpp` is the model,
  about 50 lines.
- An origin check. `QWebSocketServer` emits `originAuthenticationRequired`, and it is ours to answer. Without it any
  page can connect.
- Two modules: `WebChannel` (which depends on Qt Qml) and `WebSockets`. WebChannel is not installed in the local Qt 6.12.0
  kits. WebSockets is in the WASM kits (`C:/Qt/6.12.0/wasm_multithread/lib/cmake/Qt6WebSockets`).
- A binding. `cxx-qt-lib` 0.10 has `qt_gui`, `qt_qml` and `qt_quickcontrols`, and nothing for either module. The
  shim goes in `cpp/`, the way `providers.cpp` does it. Whether cxx-qt's generated properties publish cleanly is not
  checked.

**How it fits the bridge:**

- **Good fit: control state.** `Shell` (56 attribute lines), `Viewport` (23), `Filters` (20), `Folders` (9), `Settings`,
  `KeyMap` and `Words`. Low-rate state and commands, with the same names the QML uses.
- **Poor fit: item data.** A listing sent as a `QVariantList` becomes JSON on every change. The #28861 lesson applies
  directly. Ratios and positions stay in typed binary, over HTTP or a binary WebSocket frame.
- **Out of scope:** thumbnails and video. They need HTTP with immutable cache URLs, as above.
- **Shared state.** WebChannel registers the live QML objects, so every browser would share the desktop's `Shell` and
  `Viewport`. Several browsers would need per-session objects, which the bridge does not have.

**Constraints, the first of which decides it:**

- **Topology (decisive).** The web client would exist only while the desktop GUI runs. The goal is a headless server
  with no UI, so this rules WebChannel out.
- **No HTTP server in the repo.** `Cargo.lock` has no `axum`, `hyper`, `tiny_http` or WebSocket crate. WebChannel is
  only the WebSocket half. The Qt HTTP Server module (technology preview) is not installed either.
- **Secure context.** A `ws://` connection from an `http://<LAN-IP>` page is allowed, so this avoids the
  `SharedArrayBuffer` limit that threaded Qt-WASM hits. Over HTTPS it has to be `wss://`.
- **Licence.** Qt's modules are LGPL. `qwebchannel.js` ships in the web bundle, so the bundle carries Qt's licence
  notice.

**The route:** a Rust WebSocket server speaking JSON built from `pg-client`'s `Event` and `Command` types. Both ends are
Rust with serde, so the protocol needs no generated types. It is headless, typed, and fits the state model. The cost is
the protocol. The controller already maps events to commands, so most of the work is serialisation.

**Where this leaves it:** WebChannel is out for the web gallery. Build on a Rust server and derive the protocol from
`pg-client`.

## Open points

Each point was investigated, then validated by two batched reviewers (an evidence lens and a design lens) and a cross-point
critique. Status: *Decided* means approved in review; *Proposed* means a recommendation survived validation; *Open* means it
did not settle.

| Point | Status | Decision or proposal | Still open |
|---|---|---|---|
| P1 Preference scope | Proposed; mostly decided | Per user: `show_borders`, last file, session listing, saved views. Per device: `row_height`, `dev_mode`, `yandex_folder`. `write_back` server-wide, owner-changeable, on by default. | No `desktop.json` exists in code, although `REFACTOR_PLAN.md:785` records the split as done (`crates/desktop/src/settings.rs:1-3`). |
| P2 Deletes and moves | Decided | Delete and move are in version 1, once quarantine exists. Library folder removal and rating-driven sort are membership changes in v1. | Quarantine is not built (`REFACTOR_PLAN.md:511`); it is the first piece of work. |
| P3 View-state split | Proposed (H on blockers) | A plain `ViewState` struct for user-visible state, validated on read. Exclude `initial`, `StartGate` and `Deadlines`; rebuild timers on reconnect; add scroll to client state. | Where the `Listing` spec sits; mid-start snapshots; serde compile not checked. |
| P4 Snapshot lifetime | Proposed (L–M) | Released after disconnect, with a grace period; 10-minute idle backstop; cap of four per client. | The grace length for a sleeping phone; the client key; every number is unmeasured. |
| P5 Desktop cutover | Proposed (M) | In-process timings, then server and conformance suite, benchmark, cutover. The in-process server beside the desktop is rejected. | Whether writes move in the same step as reads (otherwise two writers). |
| P6 Video element removal | Open | Never change or clear the source while the player is Preparing or Prepared. The pause-first fix is rejected: pause is ignored in Preparing. | Video is work in progress. The candidate fix is unverified (test E1c). The served-URL source. If the fix fails, a Qt fork or a browser-native element. |
| P7 HTTPS trust | Decided (certificate); gate open | Private CA certificate with the LAN name and the IP, configured; `wss://` only. | Phone trust (Chrome on Android); `crossOriginIsolated` with a bypassed certificate; Emscripten version match (the spike used 5.0.5). |
| P8 Control and data split | Proposed (M) | Member table and grep test first; the seam at the protocol boundary; a poisoned-stub run before any bridge is rewritten. | Per-member labels are provisional. The desktop controller reads app state at start (`crates/desktop/src/controller.rs:39, 51, 55`). The table misses three `video_source` members and one `failed_files` member. |
| P9 Layout threading | Decided: Web Worker | Layout runs in a worker. | Measure to set the budget. |
| P10 Thumbnail transport | Proposed (M) | The async server, which gives HTTP/2 natively. | The TLS library (see Server); whether the WebSocket batch is still needed on the LAN; time-to-fill on a phone, measured. |
| P11 Phone spike, touch, decode | Decided: Android only | A desktop Chromium pre-screen first, then the phone gate after the trust test. The viewer preloads eight neighbours, so the decode budget counts eight. | Pass budgets, measured first (see Testing). |
| P12 Server stack | Decided: async | `axum` on `tokio`. | The TLS library: `rustls` or `native-tls`. `native-tls` has no server-side ALPN on Windows. |
| P13 CLI writers | Decided | While the server runs, the CLI writes through it; otherwise it writes directly. No refusal. | None. |
| P14 Login | Decided: login, not pairing | The owner sets a password at first run; the phone logs in and gets an httpOnly cookie bound to a revocable device record. Works while the desktop is closed. | Password storage and hashing; lockout and rate limits; the reset path; no QR code (decided). |

**Decided in the latest review, outside the table:**

- Yandex is in version 1.
- A server-side folder browser can add library folders, from the phone.
- `PLAN.md:17` is updated to put the browser client in scope.
- Terms: "saved session" for the catalog's session, "stream" for the event numbering, "launch token" for the desktop's per-start token.
- The budgets and the phone pass budgets are set by measurement.

**Not yet investigated** (the validation found these gaps; none is decided):

- The server video pipeline: stream-copy to fMP4 or HLS, real-time HLS with NVENC, one ffmpeg per playback.
- Event retention and resume: how long events are kept, what `resume(seq)` replays, and gap handling.
- The handshake and the build-id reload, including how a stale cached bundle is avoided.
- Mutation concurrency and key-repeat rotation, and the `VersionMismatch` notice.
- Endpoint coverage: tree, item detail, folder browse, Yandex, scan and job status, inbox, settings, saved views.
- Item record shape, thumbhash delivery, and the select-all wire format.
- Auth beyond the handshake: CSRF for cookie-authenticated POSTs, and the SameSite value.
- LAN safeguards and certificate tooling: renewal, and a DHCP address change.
- Windows lifecycle: start at logon, the single-instance lock, supervision, and whether low I/O priority affects blocking reads.
- Serving the built frontend: compression and cache headers for the 27.7 MB `.wasm`.
- How queued thumbnail misses are scheduled, reported and retried.
- The Qt licence notice for the bundle, and the Emscripten version match.
- Per-field mutation rules, checked against `gallery/edits.rs`.

## Open checks

- **Qt for WebAssembly video (work in progress; open).** The grid runs in the spike; video does not. Test E1c, to run in the Chromium spike on
  127.0.0.1: call `play()`, change the source before the `playing` event, then assert that the element survives and no
  `AbortError` is raised. The candidate fix is to defer source changes until Started, Paused or Stopped, and it is unverified.
  Then:
  - measure the cold start from a PWA cache on a mid-range phone;
  - measure image decode time in WASM;
  - try touch (QML is touch-first; the viewer has a `PinchHandler` at `Viewer.qml:314` and a touch-only `DragHandler`
    at `Viewer.qml:332`).
- **The gate, over HTTPS.** Re-run the spike with the trusted LAN certificate and WebSockets on the phone's browser (see
  "Reusing the QML"). Confirm `crossOriginIsolated` is true on the phone before testing threads.
- **Bridge under the state model.** Under the State model the browser build adds a data-layer transport, not a network
  call per member. Check this with a stub data layer before any bridge is rewritten.
- **WASM layout on a million ratios,** on the main thread and in a worker. It decides whether the worker is needed.
  *Decided: a worker.* Measure to set the budget, not to choose the design.
- **Batch stream against HTTP/2** with the trusted LAN certificate, on a phone, for time to fill a screen of thumbnails.
  Deferred until the thumbnail front is decided (P10).
