# Web client

How a browser client for the gallery should be built, from reading the source of the web galleries that exist
(2026-11-01). Nothing here was built or run: every finding comes from the code at the commit below, or from the
project's issues. It answers the questions left open when a web client was first discussed: which client stack,
how thumbnails get past the browser's connection limit, how video plays, and what the server must send.

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

- **Client: TypeScript with Svelte 5, `layout::justify` compiled to WASM.** Every production gallery has a
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
- **Urocissa's query snapshot** is the closest to our `list_files(folder, recursive, Query, Sort)`. The server
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
  map, committed to the repo, and CI fails on a diff (`.github/workflows/ci.yml:329-339`). Spacedrive itself has
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
  - `list_files(folder, recursive, Query, Sort)` frozen under an id (Urocissa).
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
- **Layout:** `layout::justify` (60 lines, no dependencies) built to WASM with wasm-bindgen and run in a worker
  over the whole listing. It reruns on resize, so the grid matches the desktop exactly.
- **Scrolling:** our own scroll coordinate scaled under the element-height cap, rows windowed by binary search
  (HomeGallery), images mounted only once scrolling slows (LibrePhotos).
- **State:** positions in typed arrays, selection as id sets and index ranges, select-all on the server. No
  per-item reactive objects.
- **Viewer:** PhotoSwipe or our own, with a preview under the full image and neighbours preloaded. hls.js for
  video.

## Open checks

- **Qt for WebAssembly on a phone.** QML is touch-first, and the viewer already has `PinchHandler` and a
  touch-only `DragHandler` (`Viewer.qml:261`, `:282`). Qt 6's WASM page lists no touch limits, but Qt Multimedia
  is a technology preview there. Build one QML screen for WASM, then:
  - measure the cold start from a PWA cache on a mid-range phone;
  - measure image decode time in WASM;
  - try touch and video.
  
  Every bridge object is a network client in either stack.
- **WASM layout in a worker against plain TS** on a million ratios. It decides whether the worker is needed, not
  the stack.
- **Batch stream against HTTP/2** over `tailscale cert`, on a phone, for time to fill a screen of thumbnails.
