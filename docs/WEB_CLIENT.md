# Web client

How a browser client for the gallery should be built, from reading the source of the web galleries that exist
(2026-11-01). Nothing here was built or run: every finding comes from the code at the commit below, or from the
project's issues. It answers the questions left open when a web client was first discussed: which client stack,
how thumbnails get past the browser's connection limit, how video plays, and what the server must send.

Reevaluated 2026-11-06, after the refactor: the QML question is answered in "Reusing the QML", and the client
section now uses `pg-client`, which the refactor made wasm32-clean. That pass read the code and the docs. The one build run was
`cargo check -p pg-model -p pg-client --target wasm32-unknown-unknown`, and it passes. A Qt-for-WASM spike was built
later the same day, with Qt 6.12.0 and Emscripten 5.0.5 (see "Reusing the QML"). It runs the real grid in a browser;
video does not play yet. The older Qt facts come from the Qt wiki and the CXX-Qt guide, not from Qt 6.12.

| Project | Stars | Server / client | Commit read |
|---|---|---|---|
| Immich (+ `justified-layout`) | 118k | NestJS / SvelteKit, layout in Rust→WASM | `b7510e7` (`6d5d2df`) |
| PhotoPrism | 40k | Go / Vue | `fe94b42` |
| Ente (web) | 30k | Go / Next.js, logic in Rust→WASM | `54e4e209` |
| LibrePhotos | 8k | Django / React | `eefb8d5` |
| Photoview | 6.6k | Go + GraphQL / React | `4abdab7` |
| Nextcloud Memories (+ go-vod) | 3.9k | PHP / Vue, Go transcoder | `dbf9f4e1` |
| PiGallery2 | 2.3k | Node / Angular | `e92fe15` |
| HomeGallery | 1.2k | Node / React | `5b6232c` |
| Spacedrive | 39k | Rust / React | `26b57e1` |
| lap | 4.2k | Rust (Tauri) / Vue | `f40520f` |
| Photofield | — | Go / Vue + OpenLayers | `d7374a9` |
| Urocissa | 300 | Rust (Rocket) / Vue | `102025d` |
| SPIS | 212 | Rust (axum) / htmx | `ceedbb9` |

Candidates came from GitHub topic search by stars (`photo-gallery`, `photos`, `photo-management`,
`google-photos`, `photography`), awesome-selfhosted's Photo Galleries list and Google.

## Verdict

- **Client: TypeScript with Svelte 5, `pg-client` compiled to WASM for layout and logic.** Every production gallery has a
  TypeScript client. Rust-backed ones too (Spacedrive, lap, Urocissa). Immich runs exactly this stack, and Ente
  shows Rust→WASM inside a TS app works in production. No gallery of any size has a Leptos or Dioxus client.
- **The hard problems are the same in every stack, and none is solved by the framework:**
  - the browser's maximum element height;
  - per-item reactive state;
  - thumbnails held up behind the 6-connection limit;
  - video the browser can't play.
  
  Every project writes its own grid virtualization. So the weak virtual-list ecosystem in Leptos is not the
  argument against it. The arguments are PhotoSwipe and hls.js, the TS iteration loop, and the absence of
  precedent.
- **What we can do that none of them does,** because we already have every aspect ratio and versioned thumbnail
  keys:
  - exact geometry for the whole listing up front;
  - a scrollbar that is right before anything has loaded;
  - thumbnails sized for devicePixelRatio and cached `immutable`;
  - on-demand remux and HLS for MKV/AVI instead of transcoding ahead.
- **Reusing the QML: possible for the views, not for the app.** The `.qml` files would carry over nearly unchanged. What
  does not carry over is the bridge behind them (about 220 properties, invokables and signals), the native glue, and the
  build. Each is a rewrite, not a port. Stay on Svelte and TypeScript. The Qt-WASM spike under "Reusing the QML"
  shows the real grid running in a browser, so the views are not the blocker. Video and the server side still are, and
  the Svelte path does not wait on either.

## What everyone sends, and what goes wrong

### Lists and layout

| Project | Up front | Layout | What breaks |
|---|---|---|---|
| Immich | per-month counts | WASM, main thread, per day group | heights guessed (`layout-support.svelte.ts:10-18`), corrected on load; 70k-asset month froze the page 18 s building reactive objects (#28861) |
| Memories | per-day counts + first ~50 photos | npm `justified-layout`, per day | 4:3 placeholder rows, corrected on load |
| LibrePhotos | per-day counts | JS justified | placeholders at aspect 1, rows reflow |
| Urocissa | `{id, count}` of an immutable query snapshot (`get_prefetch.rs:262-286`) | worker, min-badness, 20-item blocks | forced break every 20 items; 2400 px/block estimate |
| lap | nothing | JS | 100×100 squares that reflow as chunks arrive (`common/layout.ts:14-15`) |
| HomeGallery | the whole catalog (~1 KB/item raw) | JS, whole result, windowed rows | 200k items on Android, 400k on a 16 GB desktop (#134) |
| PiGallery2 | the whole folder (~180 B/item packed) | JS, append-only | page grows without bound; 3k-subfolder folders slow (#125) |
| PhotoPrism, Photoview | pages of 156 / 200 | square / flexbox | no full-library scrollbar (#2892, #303) |

- **Wire formats:**
  - Immich sends each month as columnar JSON, about 200 B per asset, built by Postgres itself.
  - Memories sends about 15 JSON fields per photo, also about 200 B, batched as `GET /api/days/1,2,3`.
  - PiGallery2's short keys and string tables cut a listing to 11.9 MB from 33 MB, but gzip alone took it to
    1.5 MB (#437). Compression comes first.
- **Reactive state is the real cost.** Immich's #28861 froze for 18 s with only 54 `<img>` on the page:
  JSON.parse took 15 ms, and the rest was one Svelte reactive object per asset (`timeline-day.svelte.ts:171-174`).
  Its selection keeps whole asset objects in a reactive map, and select-all loads every month (#32179, frozen at
  313K). Positions and selection belong in typed arrays and id sets, outside the framework's proxies. That holds
  for Leptos signals as much as Svelte runes.
- **Urocissa's query snapshot** is the closest to our `list_grid(folder, recursive, Query, Sort)`. The server
  freezes the filtered, sorted list under an id, and the client fetches ranges of it by index.

### The maximum element height

Three projects have it open or worked around:

- Immich #16788, #30061, #31358;
- Memories #1715;
- Urocissa's logical-to-physical scroll mapping (`useHandleScroll.ts:18-97`).

Firefox caps an element near 17.2–17.9 M px and Chromium near 33.5 M px. Past that, Firefox collapses the element
to 0 and the grid goes blank. A million items in a justified grid at about 5 per 235 px row is about 47 M px. The
scroll position has to be our own coordinate, scaled onto a spacer below the cap, from the start.

### Thumbnails and the connection limit

| Project | Sizes | URL / caching | Placeholder | DPR |
|---|---|---|---|---|
| Immich | 250 px WebP, 1440 px JPEG preview | `?c=<thumbhash>`, `private, max-age=86400` | thumbhash on a canvas | no |
| PhotoPrism | 224/500 squares, fits to 7680 | SHA1 + size, 30 days immutable | none | viewer only |
| Photoview | one 1024 px JPEG | 1 year immutable | blurhash | no |
| HomeGallery | 128–1920 JPEG, at import | SHA1, immutable (`server/src/app.js:69`) | dominant colour | yes |
| Memories | 256/512 | etag + index time | none | viewer only |
| PiGallery2 | 320–2160 WebP, on demand | unversioned, 1 day | none | no |
| LibrePhotos | 250/500 high WebP | **no Cache-Control** | colour + blurred 250 px | no |

- **Nobody uses HTTP/2 or TLS on the server.** All of them speak plain HTTP/1.1 and leave TLS to a reverse proxy,
  and they live with 6 connections per host.
- **Mitigations in use:**
  - Immich aborts off-screen `<img>` fetches from a service worker (PR #16893, which cites the limit). That needs
    a secure context, so it does nothing on `http://<LAN-IP>`. Immich #27585 shows the failure: hover-video
    requests held all 6 sockets for 50–115 s and thumbnails stalled.
  - LibrePhotos mounts full images only once scrolling slows, with a 2500 px buffer ahead and 100 px behind.
  - lap batches 4 thumbnails per IPC call, base64 in JSON.
  - Spacedrive multiplexes every event over one shared SSE stream, because one stream per subscription "exhausts
    the pool and stalls RPC POSTs" (`packages/ts-client/src/transport.ts:236-240`).
- **Memories' `multipreview`** (`lib/Controller/ImageController.php:139`) is the design to copy:
  - A worker queues misses for 20 ms or 20 requests, then sends one POST.
  - The server streams back, per image, a small length-prefixed JSON header and the bytes, smallest first.
  - The worker resolves each image as it arrives and stores it in Cache Storage under its single-image URL, so
    later loads are plain cache hits.
  
  It was built to save PHP per-request overhead, but the same framing gets past the connection limit on plain
  HTTP.
- **Costs to avoid:**
  - PiGallery2 stats the original on the photo disk for every thumbnail request, and checks up to 5 thumbnail
    files per photo on every listing. On an HDD, that made the first open of a 10k folder take 3 minutes (#613).
  - Spacedrive re-encodes a PNG per request and sends `no-cache` on a versioned URL
    (`apps/server/src/main.rs:228-279`).
  - Thumbnails must be served from the cache alone, never touching E:.

### Viewer

- **PhotoSwipe** in Memories and Ente; CSS-transform zoom everywhere else.
- **Loading order:** Immich goes thumbnail, then preview, then original on zoom. HomeGallery stacks a
  quarter-size preview under the right-size image, sized for zoom × DPR, and preloads the neighbours.
- **Zoom stops short:** at the 1440 px preview (Immich, HEIC/RAW without full-size generation) or at 1920 px
  (HomeGallery #165, #94).
- **Formats the browser can't decode:** PiGallery2 sends RAW/HEIC originals that the browser can't show. Memories
  converts them to JPEG q85 with ETag/304.
- **Nobody serves zoomed regions.** Our region decode beats all of them.

### Video

| Project | Policy |
|---|---|
| Immich | serve H.264 4:2:0 in MP4/MOV/WebM/MKV as-is with ranges; else background job to 720p H.264. New, off by default: real-time HLS, 2 s fMP4 segments, prewarming, hls.js |
| Memories (go-vod) | instant playlist of made-up 3 s segments from the duration; client sends `mediaCapabilities`; H.264 or a supported codec is stream-copied along cached keyframes, so MKV/AVI with H.264 is a copy; else encode 1–4 segments ahead, renditions 480p–2160p capped at source size and 80% of its bitrate |
| LibrePhotos | exact codec string checked with `canPlayType`; playable originals with ranges; else live fMP4 transcode (unseekable) plus a seekable copy to a bounded cache |
| PhotoPrism | M2TS remuxed; else whole-file transcode before playback |
| Photoview, PiGallery2, HomeGallery, Urocissa | transcode everything ahead to 720p–1080p H.264 |

- **Transcoding ahead doesn't fit 21 TB on an HDD.** HomeGallery users can't watch videos until the job finishes
  (#145), and PiGallery2 shows an error for MKV and AVI until then.
- **go-vod's model fits us:** remux what the browser can decode, HLS-transcode the rest on demand. It throttles
  ffmpeg with SIGSTOP and SIGCONT, which Windows lacks, so we'd pace it through our own ffmpeg-next pipeline
  instead. Its known problem is two ffmpeg processes starting at once on a quality switch (#1560).
- **Grid video tiles must release their player:** pause, clear `src`, call `load()`. Without that the browser
  runs out of media players (LibrePhotos #970, #2026).

### Realtime and auth

- **Live updates:**
  - PhotoPrism has a WebSocket bus that patches loaded items in place.
  - Immich sends socket.io events with asset objects or id lists, applied every 2.5 s.
  - HomeGallery uses SSE, which on HTTP/1.1 permanently takes one of the 6 sockets.
  - The rest poll.
- **Auth:** an httpOnly SameSite=Lax cookie lets `<img>` and `<video>` load with no token (Immich, LibrePhotos).
  A token in the path makes URLs unique per session and defeats caching (PhotoPrism). A header token forces every
  image through fetch and blob URLs (Ente, Urocissa).

### Rust and TypeScript together

- **Ente's Rust→WASM:** crypto, dedup and similar logic, built with wasm-bindgen, tsify and serde-wasm-bindgen, via
  `wasm-pack --target bundler`. The pain they record is all build-side:
  - webpack, not Turbopack;
  - a lint rule that fails the build if WASM lands in the initial bundle (it must come through `import()`);
  - WASM size as a review rule;
  - a pinned nightly Rust for Chromecast.
- **Immich's `justified-layout`:** about 200 lines, `no_std`, inlined as base64, `initSync`. It takes a
  `Float32Array` of ratios and returns a `Float32Array` of positions.
- **Spacedrive's type generation** is the one to copy. specta writes `types.ts` (about 6.4k lines) with a method
  map, committed to the repo, and CI fails on a diff. Spacedrive itself has
  since dropped rspc for its own JSON-lines RPC behind an axum proxy (`core/src/infra/wire/registry.rs`).

### Photofield: the grid as map tiles

Photofield renders the whole layout on the server as 512 px map tiles.

- **Gains:**
  - requests scale with the viewport, about 8–15 tiles, not with photo count;
  - photos too small on screen are drawn as solid colours;
  - opening a photo is just a zoom into the same map.
- **Costs:**
  - every resize is a new scene and a full server relayout;
  - with no per-item DOM, hover, selection and badges need round-trips;
  - "not optimized for many clients";
  - panics on huge scenes (#54) and memory issues (#193).

It's a poor fit for rating and selecting.

## Design for us

**Server** (a second front end on a UI-free service crate):

- **Listing snapshot:**
  - `list_grid(folder, recursive, Query, Sort)` frozen under an id (Urocissa).
  - Comes with one binary blob of every item's aspect ratio (2–4 B each, about 2–4 MB per million) and its
    thumbhash, both column-wise.
  - Everything else (path, rating, flags, details) by index range.
  - Compressed (PiGallery2 #437).
- **Thumbnails:** served from the cache only, at `…/<id>@<mtime>.<rotation>.<turns>/<height>`,
  `Cache-Control: public, max-age=31536000, immutable`. A height the cache lacks gets the nearest
  (`ThumbCache::get_nearest`).
- **Batching:** a `multipreview`-style stream, with requests in viewport-priority order and cancellation.
  - Carried on the one WebSocket, which also carries events, so plain HTTP on the LAN never runs out of sockets.
  - The client stores each image in Cache Storage under its single URL.
  - With TLS (mkcert or `tailscale cert`) and HTTP/2, plain `<img>` would also do. The batch path keeps
    working without it.
- **Viewer:** a fitted image at screen × DPR, then zoomed regions; JPEG for formats the browser can't decode.
- **Video:**
  - Original with byte ranges when the browser can play it (`canPlayType` on an exact codec string).
  - Stream-copied to fMP4 or HLS when only the container is wrong.
  - Real-time HLS transcode otherwise, paced in-process, with NVENC on the 2060.
  - Nothing ahead of time.
- **Events and auth:** events patch items by id (PhotoPrism). Auth is an httpOnly cookie set by device pairing.

**Client:**

- **Stack:** Svelte 5 + TypeScript, as a static single-page app served by `gallery.exe`, with types generated by
  specta and checked in CI.
- **Layout:** `layout::justify_into` (`crates/client/src/layout.rs`; the greedy core is about 20 lines, std only)
  built to WASM with wasm-bindgen and run in a worker over the whole listing. It reruns on resize, so the grid matches the desktop exactly.
- **Scrolling:** our own scroll coordinate scaled under the element-height cap, rows windowed by binary search
  (HomeGallery), images mounted only once scrolling slows (LibrePhotos).
- **State:** positions in typed arrays, selection as id sets and index ranges, select-all on the server. No
  per-item reactive objects.
- **Logic:** `pg-client` is wasm32-clean (`clippy.toml` bans clocks, threads and files; CI checks the build). The web
  client can reuse its pure parts through wasm-bindgen: keys, sort, facets, the Fluent text, and the controller, which
  takes `now_ms` as an input. Nothing is built yet. The Svelte client should decide per part whether to call the WASM
  or port it, and the replay format (`(now_ms, Event)` lines) lets it test the port against the desktop's traces.
- **Viewer:** PhotoSwipe or our own, with a preview under the full image and neighbours preloaded. hls.js for
  video.

## Server (draft)

The server is what the web client talks to. It owns the catalog, the thumbnail cache, the jobs and the file access.
It has no window and no Qt. This section is a draft: the open decisions are at the end.

**What the repo has to build on:**

- `pg-app` is the typed API over the catalog, thumbnails, jobs, library and Yandex (`crates/app/src`, several dozen public
  functions, most in `library.rs`). The desktop window reaches data only through it, so the server can too.
- `pg-client` has the `Query`, `Listing` and `Sort` types and the `Event`/`Command` enums, and `pg-model` has the
  shared types specta already generates for TypeScript.
- `crates/gallery` builds without Qt when its `desktop` feature is off (`crates/gallery/Cargo.toml`). A server can be a
  `serve` subcommand behind a `web` feature, next to the window.
- The repo has no async runtime and no async code. Thirteen files spawn threads, and `pg-app` is blocking (rusqlite,
  crossbeam channels). Nothing in `Cargo.lock` is an HTTP or WebSocket crate.

**Crate:** a new `crates/web` (`pg-web`), with no Qt dependency. It depends on `pg-app` and `pg-client`, and holds the
HTTP and WebSocket server, the routes, the cache headers and the auth. `crates/gallery` gets the `serve` subcommand.
The built Svelte bundle is served from a directory first, and embedded later.

**Stack (open).** Two options:

- **Async: `axum` on `tokio`.** Mainstream, with WebSocket support built in. It would be the project's first async
  code, and every `pg-app` call would need `spawn_blocking`.
- **Sync: `tiny_http` with `tungstenite`, a thread per connection.** Matches the thread model the repo already has, and
  adds no runtime. Byte-range handling for video is written by hand either way, so this costs little.

Lean: sync. The batch stream and the events share one WebSocket, so plain HTTP never runs out of sockets, and a
handful of browsers need no more threads than that. Both options leave TLS to a reverse proxy, or to `tailscale cert`
later.

**Endpoints (sketch):**

- `POST /api/listings`: a `Query` and `Sort`. Freezes the result under an id and returns the count.
- `GET /api/listings/{id}/ratios`: the aspect ratios as one binary column, 2 to 4 bytes each.
- `GET /api/listings/{id}/items?from=&to=`: the rest of each item as columnar JSON, gzip-compressed.
- `GET /api/thumbs/{file}@{mtime}.{rotation}.{turns}/{height}`: cache only, `Cache-Control: public, max-age=31536000,
  immutable`. A missing height gets the nearest one (`ThumbCache::get_nearest`).
- `GET /api/files/{id}/view?w=&h=`: a fitted image, or the original bytes with `Range` for video.
- `POST /api/pair`: device pairing, which sets the httpOnly cookie.
- `GET /ws` (WebSocket): events as JSON, and the thumbnail batch as binary frames with request ids, in viewport order,
  with cancellation.

**Behaviour the server must have:**

- **Snapshots expire.** A listing is dropped after a set idle time, and the client gets an error it can recover from by
  creating a new one.
- **The cache is the only source for thumbnails.** A miss is queued, never read from the originals on the request path.
- **Auth.** The WebSocket checks the `Origin` header, because the cookie is sent with it and cross-site pages can open
  it otherwise. The server binds to the LAN address only when asked.

**Open decisions:**

- **Stack:** async or sync. Lean sync, above.
- **Shared catalog.** The desktop and the server would read and write one SQLite catalog, and the catalog uses WAL
  (`close()` merges it). Whether two processes can write it safely is not checked. This needs a check before the server
  runs beside the window.
- **Snapshot lifetime** and the error the client sees when one has expired.
- **Pairing UX:** a code shown in the desktop window, or a login page.
- **Packaging:** one `gallery` binary with the `serve` subcommand, or a separate binary.

## Reusing the QML

**What the QML is made of** (the code at the commit above, `crates/desktop`):

- **Views:** 40 QML files, 3,796 lines, in the `pg.gallery` module. `Theme` and `Ratings` are singletons.
- **Bridge:** 14 Rust files register the types the QML uses, with `#[qml_element]`. Their members are
  `gallery.rs` (50), `shell.rs` (48), `viewport.rs` (24), `yandex.rs` (21), `filters.rs` (19), `library.rs` (18),
  `folders.rs` (11), and five more files with 2 to 5 each. Each member reads in-process Rust state: the catalog,
  the thumbnail cache, the job queue. In a browser every one of them is a network call.
- **Qt modules:** `Quick`, `Controls.Fusion`, `Layouts`, `Window`, `Multimedia` (`MediaView.qml`: `MediaPlayer`,
  `VideoOutput`, and animation frames through the same `VideoOutput`), and `Dialogs` (`FileDialog` in `Main.qml`,
  `LibraryPanel.qml` and `YandexPanel.qml`).
- **Native glue with no browser equivalent:** `cpp/providers.cpp` (a `QQuickAsyncImageProvider` over the Rust decoders),
  `cpp/frame_sink.cpp` (Rust threads feeding a `QVideoSink`), `cpp/gpu_device.cpp` (the app's D3D11 device, shared
  with Qt Quick and the player), and the screenshot and live-reload helpers. The web needs replacements for each.

**What is already in place:** `crates/desktop/tests/qml/stubs/pg/gallery/` has stand-ins for 7 of the 14 bridge types
(117 lines in all), and `tools/qmltest.py` runs the real views against them. So the views depend on names, not on the
Rust implementations. That is the seam a web backend would plug into. It covers 7 types, though, and the stubs hold
only what the tests read. A full `pg.gallery` for the browser has to implement all of the surface.

**What blocks it:**

- **Build target.** The shared crates build for `wasm32-unknown-unknown` (`Justfile:78`, `ci.yml:224`). Qt for WebAssembly
  and CXX-Qt need `wasm32-unknown-emscripten`, with POSIX threads and an Emscripten version that matches the Qt build.
  The [CXX-Qt WebAssembly guide](https://kdab.github.io/cxx-qt/book/concepts/wasm-builds.html) gives the version
  pairings. This repo has no such build, and the desktop crate already uses `cxx-qt` 0.10 (`crates/desktop/Cargo.toml`).
- **Threads and secure context.** Qt's threaded build needs `SharedArrayBuffer`, which a browser only enables on a
  cross-origin isolated page. The
  [Qt status update of June 2024](https://lists.qt-project.org/pipermail/development/2024-June/045385.html) says threads
  need that mode. Isolation needs a secure context, so on `http://<LAN-IP>` only a single-threaded build would run.
  That is the same constraint the thumbnail batching already works around.
- **Video.** Qt Multimedia on WASM plays through an HTML5 `<video>` element (see the spike below). A clip bundled as a
  `qrc` resource is copied to a `file:///tmp` URL, which the browser rejects, so the source has to come over HTTP. Over
  HTTP the `<video>` element was removed from the page during start-up and `play()` was aborted (`AbortError`). The cause
  is not found. Video is the blocker for `MediaPlayer` in a browser. The web video path outside Qt, which this doc already
  prefers, is the fallback.
- **Files.** `FileDialog` is used in three places. Browsers pick files their own way, and the library's folders live on
  the server, so these dialogs become a server-side folder browser whatever the stack. Not tested.
- **Size and start-up.** Measured on the spike: 27.7 MB of `.wasm` (9.6 MB gzipped), 294 KB of JS (67 KB gzipped), and a
  13 KB loader. Cold start on a phone is not measured.

**The spike, and what it found.** A scratch project (`wasm-spike`, not committed) builds one screen for WASM with Qt
6.12.0 and Emscripten 5.0.5, the versions Qt's WebAssembly page names. It uses the real `GridRow` and `GridCell`, with
`Theme` and `Ratings` as singletons, over 400 generated rows of five cells, plus a `MediaPlayer` on a short MP4. The
test ran in desktop Chromium, with the server sending the cross-origin isolation headers.

- **Build:** works with the multithreaded kit (`wasm_multithread`), the only one with Multimedia. Two fixes were needed:
  `BorderOverlay.qml` had to be in the QML file list, and the singleton flag had to be set before `qt_add_qml_module`.
- **Layout:** the 400 rows lay out at the right geometry (150 px rows, widths from the aspect ratios). The console was
  clean after the delegate fix.
- **Images:** none. The spike passes no thumbnail URLs, since the image providers are Rust code and not in the spike.
  The cells show only what `GridCell` draws: its overlay colours, including the `#ff4ad8` square for unmarked borders.
- **Video:** not playing. The status and error log lines were `player media status 6`, then `AbortError: The play()
  request was interrupted because the media was removed from the document`. An earlier run hit `NotSupportedError`, with
  the source a `qrc` URL.
- **Not run:** the single-threaded kit (it has no Multimedia), the `http://<LAN-IP>` secure-context check, touch on a
  phone, and cold start.

What this settles: the QML views run in a browser under Qt for WASM, so the `.qml` files are not the obstacle. What it
leaves open is video, the server side (images and data come over the wire, not from Rust in-process), and the phone
checks. The cost of a second `pg.gallery` implementation (about the size of the bridge above) is the next question if
video cannot be fixed. The Svelte path does not wait on it.

## WebChannel over WebSockets

One way the desktop app could serve its bridge objects to a browser: Qt WebChannel over a `QWebSocketServer`. The
browser gets a proxy for each object: properties, signals and invokables. This is ruled out. The goal is a headless
web gallery, not a remote view of the desktop app, and WebChannel needs the app running. The facts are kept for
reference, from the Qt 6.12.0 source (`C:\Qt\6.12.0\Src\qtwebchannel`, `qtwebsockets`).

**What Qt publishes.** Every property, with its notify signal. Every public `Q_INVOKABLE` method and public slot.
All signals and enums. Property changes are batched on a timer, 50 ms by default (`propertyUpdateInterval`, with
`blockUpdates` to pause). Arguments and results are JSON (`qmetaobjectpublisher.cpp`).

**What it needs that Qt does not give:**

- A WebSocket transport. The library has none; `examples/webchannel/standalone/websockettransport.cpp` is the model,
  about 50 lines.
- An origin check. `QWebSocketServer` emits `originAuthenticationRequired`, and it is ours to answer. Without it any
  page can connect.
- Two modules: `WebChannel` (which depends on Qt Qml) and `WebSockets`. Neither is ticked in the Maintenance Tool.
- A binding. `cxx-qt-lib` 0.10 has `qt_gui`, `qt_qml` and `qt_quickcontrols`, and nothing for either module. The
  shim goes in `cpp/`, the way `providers.cpp` does it. Whether cxx-qt's generated properties publish cleanly is not
  checked.

**How it fits the bridge:**

- **Good fit: control state.** `Shell` (48 members), `Viewport` (24), `Filters` (19), `Folders` (11), `Settings`,
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
  only the WebSocket half. The Qt HTTP Server module (technology preview) is also unticked.
- **Secure context.** A `ws://` connection from an `http://<LAN-IP>` page is allowed, so this avoids the
  `SharedArrayBuffer` limit that threaded Qt-WASM hits. Over HTTPS it has to be `wss://`.
- **Licence.** Qt's modules are LGPL. `qwebchannel.js` ships in the web bundle, so the bundle carries Qt's licence
  notice.

**The route:** a Rust WebSocket server speaking JSON built from `pg-client`'s `Event` and `Command` types, which specta
already generates. It is headless, typed, and fits the design above. The cost is the protocol. The controller already
maps events to commands, so most of the work is serialisation.

**Where this leaves it:** WebChannel is out for the web gallery. Build on a Rust server and derive the protocol from
`pg-client`.

## Open checks

- **Qt for WebAssembly, the spike in "Reusing the QML".** The grid runs; video does not yet. Still open: why the
  `<video>` element is removed during start-up (check whether `VideoOutput` teardown removes it, or bind `videoOutput`
  another way); then:
  - measure the cold start from a PWA cache on a mid-range phone;
  - measure image decode time in WASM;
  - try touch (QML is touch-first, and the viewer has `PinchHandler` and a touch-only `DragHandler`, `Viewer.qml:308`).
  
  Every bridge object is a network client in either stack. The spike is the S-14 "Qt-WASM screen" that
  `REFACTOR_PLAN.md` left optional; it has now been run, without thumbnails.
- **WASM layout in a worker against plain TS** on a million ratios. It decides whether the worker is needed, not
  the stack.
- **Batch stream against HTTP/2** over `tailscale cert`, on a phone, for time to fill a screen of thumbnails.
