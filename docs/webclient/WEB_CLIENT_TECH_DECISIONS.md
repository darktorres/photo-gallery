# Web client: technical decisions and spike results

Recorded decisions for the open points that need no owner input (the B list), with the spike results (C list) that
checked the premises they rest on. Checked on 2026-11-14 against this repo and the local toolchain. Where a decision
supersedes text in `WEB_CLIENT.md`, `WEB_CLIENT_DESIGN.md` or `WEB_CLIENT_RECONCILED.md`, the text names it. The owner
decisions (listing model, migration 10 timing, the frontend gate, and the rest of the owner list) are still open and are
not changed here.

## Spike results (C)

| #  | Check                                                                                | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Consequence                                                                                                                                                                                     |
|----|--------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| C1 | Qt WASM WebSocket ping                                                               | `qwebsocket_wasm_p.cpp` (293 lines) has no ping, pong or timer code. The browser WebSocket API has no way to send a ping frame, but browsers answer server ping frames automatically (RFC 6455).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | The server can send pings to detect dead connections. The lack of client-side pings does not block a WebSocket. Corrects the earlier "no keepalive" conclusion.                                 |
| C2 | native-tls ALPN on Windows                                                           | **The doc's claim is wrong.** native-tls 0.2.18 has a server-side ALPN accept path on schannel (`src/imp/schannel.rs:338-367`), behind the opt-in `alpn-accept` feature (`src/lib.rs:534-565`). The feature is off in this repo.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | The argument against native-tls is weaker than the doc says. It is an opt-in feature with a blocking `accept`, and it has no tokio/axum integration. See B5.                                    |
| C3 | ureq 3.5.1 holds a 25 s request                                                      | Passed. A 25.5 s `GET` completed with 200 and no timeout. ureq 3's defaults are all `None` except `timeout_await_100` (`Some(1s)`, `config.rs:1097-1108`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | The 25 s change poll is safe on the client side with defaults. Do not set a global timeout on the poll call.                                                                                    |
| C4 | Argon2id cost on this machine (i7-8700, 12 threads, argon2-cffi 25.1.0, median of 5) | 19 MiB/t2: 0.05 s. 64 MiB/t2: 0.15 s. 64 MiB/t3: 0.21 s. 128 MiB/t3: 0.59 s (range 0.42–0.92 s).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | 1 s is not reached by any set at or below 128 MiB, t3. See B8.                                                                                                                                  |
| C5 | Chrome accepts `Secure` `__Host-` cookies over plain HTTP on a LAN IP                | **No.** Headless Chrome, one session per origin: the cookie is sent back on `http://localhost:8788` and **not** on `http://192.168.1.92:8788`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Login over the LAN needs HTTPS. TLS is not optional. See B3.                                                                                                                                    |
| C6 | Stack compiles natively (axum, tokio, rustls) and h2 limits                          | `cargo check` passes natively on Windows with axum 0.8.9, hyper 1.12.0, hyper-util 0.1.21, h2 0.4.20, tokio 1.54.1, rustls 0.23.46, tokio-rustls 0.26.6. axum's default features enable only hyper's `http1`, so h2 needs `hyper` `http2` and `hyper-util` `server-auto`. Advertised limits: max concurrent streams 200 (`hyper-1.12.0/src/proto/h2/server.rs:69`); SETTINGS_MAX_HEADER_LIST_SIZE 16 KB (`server.rs:40,76`). h2's rapid-reset guards: remote reset cap 20 (`h2-0.4.20/src/proto/mod.rs:34`, enforced `streams/recv.rs:909-921`, GOAWAY ENHANCE_YOUR_CALM) and local reset cap 50 (`proto/mod.rs:43`, `counts.rs:226`). `axum::serve` does not expose the header-list limit. Not verified: the rapid-reset fix version (from memory, not the tree). No TLS or ALPN exercised. | Use hyper-util's auto builder with a custom accept loop, not `axum::serve`, so the header-list limit can be set (B17). The build is confirmed; the rapid-reset guards are present at h2 0.4.20. |
| C7 | Repo premises (B list)                                                               | The `ureq` entry in `[workspace.dependencies]` is the one meant (cited by name, not line). `ring` and `rustls` are absent from `Cargo.lock`, so the "ureq already uses ring" claim is false. `SETTINGS` (`PRAGMA foreign_keys = ON`) is at `schema.rs:128`. `files` triggers are at `schema.rs:754-818`, not 707-728 as the reconciled doc cites. Line numbers in the reconciled doc are stale.                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Cite the current lines in any design text.                                                                                                                                                      |

## Decisions (B)

**B1. One store: `server.db`.** Owner, devices, sessions, login failures, preferences, the remembered rows and the
ledger all live in `server.db`. `auth.sqlite` is dropped. Two stores can't be updated atomically together, and the doc
already has a non-atomic real_size import across two databases (lan:84). Supersedes WEB_CLIENT_DESIGN's auth-store
placement and writes' `auth.sqlite`.

**B2. Operation ledger in `catalog.sqlite`, written in the same transaction as the mutation.** The pending/done ledger
in `server.db` (writes) can't be atomic with a catalog commit. An entry is written in the same transaction as its effect,
so there is no pending state to recover. Keep the name `request_ledger` and do not call it `ledger`: `ledger.rs` already
exists for failure ledgers (lan:97). Supersedes writes' request_ledger placement and resolves O25.

**B3. Cookies: `__Host-gallery_session` (Secure, HttpOnly, SameSite=Lax, Path=/) and `gallery_device`
(Secure, HttpOnly, SameSite=Lax, Path=`/api/auth/login`).** Lax, not Strict: an external link to a photo must still
arrive signed in (writes' argument, unanswered by identity). On logout, expire the session cookie with `Set-Cookie`
Max-Age=0 and do not use `Clear-Site-Data: "cookies"`, which removes the device cookie too and creates a new device,
losing preferences (transport's gap). Clear `"cache"` and `"storage"` only. Supersedes all three cookie designs.
**Consequence (C5):** `__Host-` cookies carry `Secure`, and Chrome will not set them over LAN HTTP. HTTPS is therefore
required on the LAN for login itself, not only for threads. The private-CA trust test on the phone gates login, not just
the HTTP/2 and threads question. Update the Status text to say so.

**B4. CSRF: `X-Pg-Client` on every POST, PUT, PATCH and DELETE.** Transport's name. The login POST is not build-gated,
so it needs a caller marker, and this header is that marker. Retire `X-Gallery-Request` (identity) and `X-Pg-Bundle`
(writes). Keep `X-Pg-Bundle` / `X-Pg-Build` only as build-identity headers, not as CSRF.

**B5. TLS: one stack, `rustls`, for the LAN listener and for ureq.** The reasons changed after C2. The doc said
native-tls cannot do server ALPN on Windows. That is false. The real reasons are these:

- rustls is async-friendly and has first-class tokio and axum integration. native-tls's server accept is blocking and
  opt-in, so axum would need a `spawn_blocking` wrapper around every TLS accept.
- Keeping two TLS stacks (rustls for the server, native-tls for ureq) doubles the trust surface.
- `ring` is not in the lock today, so the move is a real migration, not a reuse. Yandex's `PlatformVerifier` and
  revocation behaviour change with it (O33). Keep Windows trust-store verification for Yandex through a platform verifier,
  and test the revocation behaviour before merging. This is untested here.
Supersedes the reconciled doc's TLS premise and the native-tls "no ALPN" reason.

**B6. Thumbnails.**

- A miss with nothing cached: `200 image/png`, a 1x1 transparent PNG, `Cache-Control: no-store`, `X-Thumb-State: pending`.
  The ThumbHash is drawn in CSS. This is transport's answer, and writes' "returns the ThumbHash placeholder" is dropped.
- A miss at height h with a nearest cached height: `307` to the nearest height's URL, with `Cache-Control: no-store`.
  This is needed because a cached redirect for the requested URL would pin the wrong size.
- Only a 200 for the exact keyed URL gets `Cache-Control: private, max-age=604800, immutable`.
- The batch stream is dropped with the WebSocket. Thumbnails are HTTP/2 image GETs (or HTTP/1.1 over the six connections,
  if HTTP/2 is not negotiated), and queueing is driven only by `POST /api/thumbs/wants` (the want list). A GET never
  queues work (ops rule 15).
- The thumbs scope bumps after each fill's commit, batched at 250 ms, and the client re-sends its whole want list (transport).
Supersedes writes' miss-on-GET queueing and identity's rule that a GET enqueues work.

**B7. Failed writes.** A commit that fails answers `503 not-persisted`. The in-memory value is unchanged. The client
reverts its optimistic edit and shows a notice; it does not keep the unsaved value. `507 storage-full` is used only for
`SQLITE_FULL`. Supersedes transport's 507-with-memory-unchanged (it rested on a misreading of `store.rs:51-57`),
identity's 503 (which matches this), and writes' keep-in-memory rule.

**B8. Owner question O3, the Argon2id cost, has a measured answer.** On the owner's machine the options are:
64 MiB, t=3, p=1 at 0.21 s, or 128 MiB, t=3, p=1 at 0.59 s (range 0.42–0.92 s). Recommend **128 MiB, t=3, p=1**, because
the threat is an offline attack on a leaked database and per-guess cost matters. Use 64 MiB if the memory is needed
elsewhere. The 15-character minimum does most of the work. Reaching 1 s needs about 128 MiB at t=5, which is not
worth it for a login. The owner still decides the memory figure.

**B9. Handshake and loopback.** The loopback `hello` returns the protocol envelope integer, `build_id`, `catalog_id`, and a
keyed proof over a nonce, keyed with the launch token. The desktop checks the proof before it sends the bearer.
Pre-login is `GET /login` and `POST /api/auth/login` only. `hello` is loopback-only and is outside the LAN gate. The
launch file is `launch.json` (port and token) (identity), not `server.token` and `server.json`.

**B10. One route table and one error enum,** generated from one source, with the route-table test (lan:102, transport's
body-cap table). The error body is `{"error": "<kebab-case code>"}`. Route names: `POST /api/server/stop` (build-exempt on
loopback, so a stale build can be stopped; the writes cluster's `POST /api/stop` is renamed); `POST /api/jobs/{job}/cancel`;
`PUT /api/inbox/viewing/{client}`; `POST /api/folders`.

**B11. Yandex folder: a server-issued folder token.** The desktop's native picker registers its chosen path through the
server and gets a token back. Clients never send raw paths for Yandex saves. Desktop and phone use the same rule. Phone
saves are off in v1 (O30). Result folder is owner-wide (O31).

**B12. Quarantine caps: 5,000 FileIds and a 1 MiB body** (writes). **Superseded:** quarantine is dropped (owner decisions,
third round), so this cap is not in force. The arithmetic fits: 5,000 × about 40 bytes is about
200 KB. Transport's 4,096-entry and 128 KiB figures are not dropped: they are the selection caps (`WEB_CLIENT_RECONCILED.md` 1.7). With quarantine gone, no quarantine route joins the body-cap table.

**B13. Versioned remembered rows.** Settings and the session listing get a `version` column, and writes are
`set(field, value, expected_version)`. This removes the last-writer-wins rule in transport (`PUT /api/settings/{key}`,
`PUT /api/session/listing`). The `version` columns are part of migration 10, so they follow O17's timing: until that
migration ships, these rows stay unversioned, and the pre-migration window is as O17 describes.

**B14. Rebuild fixes (must land before any code).**

- Set `PRAGMA foreign_keys = OFF` outside the transaction. Inside a transaction it is a no-op (`SETTINGS` sets it ON at
  `schema.rs:128`, and each migration runs in one transaction).
- `DROP TABLE files` removes every trigger on `files`. The rebuild must recreate all of them: `files_added`,
  `files_changed`, `files_source_changed`, `files_failures_stale`, `files_failures_hidden`, plus `files_folder`,
  `files_ref` and `files_fresh`.
- Stamp the columns the listing reads but the stamp triggers miss: `ratings.rating`, `file_writes.value`,
  `border_marks` presence, and `analysis.error`.

**B15. Remembered saves.** Debounce 1,000 ms (the design's and the code's value; `deadlines.rs` asserts it). Flush on `visibilitychange` to hidden with `keepalive`. This replaces O9
(timing) and reduces the loss in O10 to what a kill inside the debounce window can cost.

**B17. Serving: hyper-util's auto builder behind a custom accept loop.** `axum::serve` can't set the HTTP/2 header-list
limit, and the LAN listener needs its own caps (connection cap, pre-auth timeout, header and body limits). Use
`hyper_util::server::conn::auto::Builder` directly with `max_header_list_size` set, and keep axum as the router on top.
Set the stream and reset limits explicitly rather than relying on the defaults above. The values are unmeasured.

**B16. Keepalive for the WebSocket.** The server sends WebSocket ping frames on an interval and closes a connection
whose pong does not come back. The client reconnects with backoff (the design's sync state machine). The vector is re-sent on
every reconnect, so no replay is needed. Ping interval and timeout are unmeasured.

## Still open after B and C

- **Owner decisions** (listing model, migration 10 timing, the frontend gate, logon task, write-back elevation, and the
  rest of O1–O45). Nothing here changes them.
- **Chrome Android** is not tested. The cookie check (C5) ran in desktop Chrome, which applies the same rule, but phone
  trust of the private CA has to be tested on the phone.
- **Qt WASM video** (E1c) is not run.
- **TLS on the phone**: rustls with the private CA, and `crossOriginIsolated` on the phone.

## Owner decisions taken so far (section A of the owner list)

Accepted as recommended, 2026-11-14:

- **O16 listing model: client-held replica (option A).** This amends the decided listing stamp (`listing_id, generation`),
  so the decided-items text in `WEB_CLIENT.md` must be amended when the consolidated doc is written.
- **O17 migration 10: after the cutover (option c).** Keeps the cutover schema-neutral. Desktop writes stay unversioned
  until migration 10 ships.
- **O4 frontend gate: time-boxed.** Run E1c (video) and the phone trust test before any bridge rewrite. If either
  fails, the separate browser frontend (the fallback) is the plan.
- **O5 logon task: always on at logon, 3 restarts at 1 minute.** The only option that matches the decided "login works
  while the desktop is closed".
- **O2 session lifetime: 30 days idle, 90 days absolute.**
- **O3 password: 15 characters minimum.** Argon2id at 128 MiB, t=3, p=1 (0.59 s measured here, B8).
- **O22 second desktop: refuse** (`desktop.lock`).
- **O43 LAN: off by default, Private profile required. O42 sleep: job-scoped keep-awake, off by default.**

Open, not yet answered:

- **O1 write-back elevation** (the reason is given in the chat; the recommendation stands unless changed).
- **O7 quarantine purge over the LAN** (disabled in v1 is the recommendation).
- **O11 Yandex from a phone** (see B11, revised below).
- **O20/O21 CLI writes and server start**, **O27 WebSocket vs long poll**, **O11/O12 view and thumbnail caching**
  (these three are being discussed).

## Revised B11: Yandex folder choice goes through the folder browser

The reconciled design needs one way to choose a folder on the server's disk. The folder browser is that way.
Yandex saves from a phone reuse it. Nothing in `docs/plans` has a folder-browser task; the decision exists only in
`WEB_CLIENT.md:33` (decided) and `WEB_CLIENT_DESIGN.md` (the open "Folder browser and write targets" question). Add
the task to the plans when the consolidated doc is written.

The desktop already has the parts. On Windows the native dialog is the system `IFileOpenDialog`, which the web can't
use. The non-Windows picker lists directories in `crates/desktop/src/picker.rs` (`show_folder`, `enter`, `up`), and the
start-folder rule is in `crates/platform/src/picker.rs:44`. The listing is inside the Qt-bound desktop crate, so the
reuse means moving the listing into `pg-platform` (or `pg-app`) as a typed function. Then:

- `GET /api/folders/browse?path=` returns the child folders of a path the server can read, with the same normalisation
  the desktop uses.
- The web folder pickers, the desktop's non-Windows picker and the Yandex phone save all call that one listing.
- The Yandex folder token (the server issues a token only for a folder it listed) is the single rule for choosing a
  save folder. Clients never send raw paths.
- The Windows desktop keeps the native dialog. Its chosen path is registered with the server to get a token.

A phone save writes onto the server's disk. The browser must not let the phone save anywhere the server can write.
Limit the phone's choices to library folders, or to one owner-configured Yandex folder. That rule is what O30 leaves
open, and it has to be decided before phone saves are allowed.

## Owner decisions, second round

- **O1 write-back: no elevation (option c).** Enabling and disabling write-back need no password. Option (c) as written
  also means revoke needs none. If bulk revoke should keep a password, say so; that is a separate choice.
- **O30 Yandex from a phone: allowed in v1, limited to library folders.** The phone picks a folder through the folder
  browser (B11), and the server issues a token only for a library folder it listed. Open consequence: a Yandex save
  writes new files into a library folder, and the scanner then imports them as photos. Confirm that is wanted.
- **O27 push channel: WebSocket, carrying the vector model (accepted).** Replaces the long poll. Reasons: Qt for WASM
  loads HTTP response bodies whole (`qnetworkreplywasmimpl.cpp:259`), but its WebSocket streams through the browser's own
  WebSocket (`qwebsocket_wasm_p.cpp`); the vector model works over either transport; and the earlier "no keepalive" claim was wrong (C1).
  Costs: an Origin check on the handshake; one HTTP/1.1 socket held open, or RFC 8441 over HTTP/2 (axum enables it,
  the phone's Chrome untested); reconnect and backoff logic.
- **O9 CLI writes and the database engine: under review.** `docs/plans/DATABASE.md` already concludes that the catalog
  stays on SQLite. Multi-writer is handled by the single-writer design, not by the engine.
- **Quarantine mechanism (O10): under review.** The rename design and a catalog flag are both on the table.

## Owner decisions, third round

- **Quarantine is dropped.** A user delete sends the file to the Windows Recycle Bin. Restore happens in Explorer; the
  file returns on the next scan as a new row, with a new id (migration 10 adds AUTOINCREMENT, so ids are not reused).
- **No purge command, from anywhere.** Emptying the Recycle Bin is an Explorer action. This closes O7 and the purge
  questions in O41 and the design's quarantine section.
- **Tombstones stay, and are explained.** Removed files are not displayed. The tombstone is the phone's replica
  removal signal: a replica is a copy of the list, and without a removal instruction it keeps showing a deleted file.
  The design records that nothing stamps a deleted row, so the tombstone is the only removal signal. Retention 90 days
  (proposal), with a full resync for any client older than the oldest retained tombstone.
- Delete order: move the file to the Recycle Bin first (a file operation, outside any database transaction), then delete
  the row and commit. If the move fails, nothing changes. If the commit fails after the move, the file is in the Bin and
  the row remains; the next scan finds the path gone and the row goes to `missing`, and the removal is retried.

## Sync: incremental by default (recommendation, not yet accepted)

- The design is already incremental: a Delta carries every change since the client's seq. Tombstones exist so removals
  travel in the same delta.
- The time-based prune (90 days, design) forces a full resync on any phone away longer than that. Replace it with an
  acknowledged-seq floor: the server keeps, per device, the last seq the device applied (the device record), and prunes
  tombstones below the minimum across live devices. Revoking or forgetting a device releases its floor.
- Tombstone size, estimated not measured: about 40 bytes per row plus its index; a million deletes is tens of MB.
- What still forces a full resync, and should stay rare: a restored or rebuilt catalog (catalog_id or seq mismatch),
  bulk changes with no per-row stamp (`bulk_seq`: zone change, layout change).
- Fix at design time: `listing_key` does not exist in code yet. The design derives it from the serialised Listing, so
  any serde change would orphan every replica on upgrade. Define the key as an explicit, versioned encoding.
- Migration 10 must keep `files.id` and `seq` values, so an upgrade does not force a resync.

## Hash honesty: correctness first (decision and test)

Principle, from the owner: stored hashes must match the bytes; performance comes second. Hashing at import is already
done for every file and is not a cost concern.

**Test (this machine, D: NTFS, unprivileged USN read on the volume root handle, 2026-11-14):** a content write followed by
restoring the timestamp is recorded. The journal holds `DATA_OVERWRITE` and `CLOSE` for the file, and `BASIC_INFO_CHANGE`
for the timestamp restore. Records matched the file's identity (`st_ino` equals the file reference). Probe scripts are in
the scratchpad. The record name did not parse; the reasons are what the design needs. Not tested: writes through
memory-mapped views, hard-link writes, and the journal wrapping.

**Design (proposed, not yet accepted):**

1. Our writes: compute the hash from the bytes we write, and store it in the same transaction. Drop the deferred `None`
   at `crates/writeback/src/lib.rs:565`, which is the pixel-rotation gap.
2. External changes: read the USN journal (the plan's journal pass, step 3.9, already reads every reason). A record with a
   data reason (`DATA_OVERWRITE`, `DATA_EXTEND`, `DATA_TRUNCATION`) or a replacement (`FILE_CREATE`, `RENAME_NEW_NAME`)
   marks that file for rehash. `BASIC_INFO_CHANGE` alone does not change the bytes and does not trigger a rehash.
3. Journal gaps: a wrapped journal, a changed journal id, a journal that is off, or a volume with no USN journal triggers
   a full verification of that root (rehash every file). This is rare and expensive, and it is required for correctness.
4. Non-NTFS roots: no journal, so they need full verification on a schedule, or a stated limit. Open.
5. Cost: a rehash is one read of the changed file, about the import cost. A full verification reads the whole root:
   time is set by disk speed, not BLAKE3 (at 150 MB/s, one TB takes about 2 hours). The library size is not known yet.

**Consequence for caching:** with honest hashes, option D (content-hash key, immutable) is correct, and option C's content
ETag is not needed. Window: a file changed outside the app keeps its old hash until the journal pass runs; the plan's
USN watch sets the latency.

**Open:** non-NTFS roots (which exist?); whether journal sizing (elevated only, per the plan) is enough or gaps fall back
to full verification; the latency window; memory-mapped and hard-link writes need a test.

## Decisions, fourth round (hash honesty and the journal)

- **Non-NTFS volumes are not allowed.** A library root must be a local NTFS volume. The add-root path refuses anything
  else, and so does the folder browser (it lists only library-eligible folders). Network shares are excluded even when
  they report NTFS: a mapped share has no USN journal on the client. The rule is therefore "local NTFS, with a journal
  that answers the USN query". Non-NTFS full verification is dropped from the open list.
- **Sequencing: honest hashes first, then option D.** Option D (content-hash key) depends on the stored hash being true,
  so the USN content-change path and the write-path hashing land first. Option D's cache change and URL change follow.
- **The journal must work both elevated and unelevated.** Reading the journal does not need elevation (tested). Resizing
  does (the plan's `ResizeJournal` helper, elevated only, keeps the id). So:
  - *Elevated:* resize to cover the retention window (the plan's sizing: rate × max(verify, retention, 30 days) × 1.5,
    clamped to 32 MiB–8 GiB).
  - *Unelevated:* the journal keeps its current size. Every pass checks the retained window (`lowest_valid_usn`) and the
    journal id against the root's checkpoint. A gap in either triggers a full verification of that root, with a notice.
    Passes run often enough (the USN watch) that a gap is rare while the app runs. It can still happen after a long time
    off.
  - *Unelevated, journal inactive* (no journal on the volume, and creating one needs elevation): the app cannot verify
    hashes incrementally. Proposed: full verification of that root, and a one-time request to elevate and enable the
    journal. Options for the owner are below.
- **Open:**
  1. Unelevated and no journal: full verification plus a one-time elevation request (proposed), or refuse to verify that
     root until the journal is enabled.
  2. How long a gap the owner accepts before a full verification. The retention window the owner wants (plan default: 30
     days) decides how often unelevated machines verify.
  3. A removable NTFS drive that is replaced: the journal id check catches a new volume; the same volume re-attached keeps
     its checkpoint.

## Decided, fifth round

- **Unelevated, no journal on the volume:** full verification of that root, plus a one-time request to elevate and enable
  the journal. Accepted.
- **Retention window: 30 days.** Elevated machines resize to cover it (the plan's sizing rule, with 30 days as the floor).
  Unelevated machines verify a root in full if a gap is longer than the journal can hold. Accepted.

## Turso spike result (2026-11-14)

Spike in the detached worktree `D:/GitHub/pg-turso-spike` (uncommitted `crates/turso_spike`, `Cargo.lock` changes). Turso
0.9.0-pre.5 (the newest published; stable 0.8.3 not run). Verdict: **not adoptable now.**

- **Catalog DDL does not run as written.** `WITHOUT ROWID` needs an experimental flag, and even with it: no indexes and
  no foreign keys on those tables. The catalog uses them (`border_sides`, `pdq_pairs`).
- **No `pg_contains` function in the public API.** Every query condition in `query/src/lib.rs` uses it, so a port would
  rewrite the condition builder.
- **Missing API:** `query_map`, `.optional()`, `rusqlite::types` error variants. `FromValue` is sealed, so user types
  can't be read directly.
- **Experimental flags required:** multi-process WAL (with IOCP), ATTACH with a BLOB filename (rejected even with the
  flag), VACUUM, autovacuum.
- **Matches:** triggers, views, partial indexes, savepoints, text ATTACH, constraint error text, crash recovery.
- **Two writers:** works only with the experimental IOCP flag. The design needs no second writer (the server is the only
  writer), so this result does not argue for Turso.
- **Benchmark (2M rows, default settings, medians):** insert 0.55x of SQLite; key lookup 0.26x; point update 0.35x;
  GROUP BY 1.27x; page by key 1.47x. Mixed, and the SQLite lookup (113 µs) is ten times DATABASE.md's 11 µs, so the
  harness is not comparable to that figure. A 571 MiB `-wal` was left after close (not investigated).
- **Revisit triggers** (DATABASE.md) are not met: two-process WAL is still experimental, and the spike adds two blockers
  (WITHOUT ROWID, the missing function API).
- **Decision:** the catalog stays on SQLite (DATABASE.md's verdict stands). Worktree cleanup: owner's call.
