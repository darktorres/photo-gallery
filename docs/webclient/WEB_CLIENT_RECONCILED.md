# Web client: reconciled design (not yet converged)

Output of the reconciliation workflow (`wf_f8d5054f-26c`). Each of five clusters was reconciled from two lenses and judged. The clusters were not reconciled with each other. The validators found the conflicts below, so this document is **not a consistent design yet**. Decisions in `WEB_CLIENT.md` govern. The owner questions still need answers before the next pass.

## Conflicts between clusters (validators)

### transport

- Settings failure semantics rest on a fact the code does not support. The text says the file is written first and, on failure, the answer is 507 with memory unchanged, citing store.rs:51-57. JsonStore::update (crates/app/src/store.rs:51-57) applies change(&mut current) to memory first and then calls write_atomically; its doc says 'The change is kept in memory even when the write fails'. The required order is the reverse of what the code does, so the claim cannot cite this code as support.
- Attach is build_id only, which breaks the decided item at WEB_CLIENT.md:260 (the handshake carries a protocol version and a build id, and the desktop refuses a different protocol version). The writes cluster already requires envelope 1 plus build_id; transport must adopt that.
- Cookie is not reconciled. Transport uses one cookie, `__Host-pg_sign`, SameSite=Lax, and finds the device by the hash of its value. Identity uses `__Host-gallery_session` (Strict) plus gallery_device (Path=/api/auth/login, Strict). Writes uses `__Host-gallery_session`, Lax, one cookie. Name, SameSite and cookie count differ across the three clusters. Transport also has a real gap: its logout uses Clear-Site-Data 'cookies', which removes the only cookie, so the next login cannot find the old device and creates a new one, losing preferences. Identity's gallery_device avoids this.
- Pre-login set includes GET /api/handshake, which breaks the decided rule at WEB_CLIENT.md:274-275 (login comes before any request other than login). The LAN browser can get bundle_id from the entry page; the handshake is needed only on loopback for the desktop. Entry page naming also differs: transport says GET /, identity says GET /login, writes says GET / with index.html.
- Route names conflict. Transport resolves /api/login, /api/logout, /api/me. Identity resolves /api/auth/login, /api/auth/logout, /api/auth/session. Writes uses GET /api/handshake, which identity does not have (it has GET /api/hello with a proof). The resolution must name one set and state the device and settings routes beside it.
- Loopback discovery and proof are not reconciled. Transport and writes use server.token plus server.json, and an unauthenticated handshake. Identity uses launch.json (port and token) and GET /api/hello with a keyed proof, so a squatter on a stale port cannot take the bearer. Transport's resolution does not say the desktop never sends the bearer before the reply is checked. Identity's proof is needed for that; the proof should be adopted with the envelope in the reply.
- Write-back placement is contested three ways. Transport: gallery.json, file-first, plus an in-memory lock. Identity: server.db settings table. Writes: server.db remembered table, server scope. Only one can stand.
- Versioned sets are violated. Transport's PUT /api/settings/{key} and PUT /api/session/listing are last-writer-wins with no version. The decided item (WEB_CLIENT.md:27-28, Status) says mutations are versioned sets. Writes gives both a version from C8; transport must do the same, or the decided item must be amended.
- Thumbnail miss queuing conflicts with other clusters. Transport says the GET never queues and only POST /api/thumbs/wants queues. Identity says a miss enqueues cache work on an authenticated GET and rejects moving it behind POST. Writes says a miss queues the make keyed by (file_id, height) on GET. Transport's position is the one that keeps the GET free of side effects; identity and writes must change.
- Placeholder body conflicts. Transport: 200 image/png 1x1 transparent, no-store, X-Thumb-State pending. Writes: 'returns the ThumbHash placeholder'. Pick one; the 1x1 PNG is what the transport resolution and its miss rules rely on.
- Cache-Control is inconsistent inside transport and with identity. The resolution says the no-cache rule for regions is dropped, while identity says region and unversioned images are private, no-cache. Owner question 2 recommends private, max-age=604800, but the working value stated in the resolution is max-age=31536000. Pick one recommendation for Q2 and state the working value to match.
- Error codes are not one set. Transport's frozen set excludes version-mismatch; writes section 8 freezes it. Transport's list has no refused, plan-expired, not-persisted, request-reused, purge-lan-disabled, device-limit or not-revocable, which writes and identity use. Transport maps SQLITE_FULL to 507 storage-full; writes maps a failed commit to 503 not-persisted; identity maps a failed commit to 503. The 401 body shape ({"error": code}, used by identity) is not stated in transport.
- CSRF header is named three ways. Transport requires X-Pg-Client on every unsafe method. Writes uses X-Pg-Bundle as the non-simple header and rejects X-Pg-Client. Identity uses X-Gallery-Request. The login POST is not build-gated, so it needs a caller marker, which X-Pg-Client supplies; the others must conform.
- Ops ledger and migration 10 disagree. Transport puts a catalog ops table and the job tables in migration 10, written in the same transaction as each mutation, and uses op as the key. Writes uses a server.db request_ledger with pending and done rows and no ATTACH, and its migration 10 is only the AUTOINCREMENT rebuild plus the seq columns. The job tables are in the catalog in transport and in server.db (job_reports) in writes. The field op is retired by writes in favour of request_id.
- Migration 10 timing is opposite across clusters. Transport owner question 8 recommends shipping 10 with the cutover, and transport's post-flip desktop writes depend on the guards. Writes Q-T2 recommends the release after the cutover. Both cannot hold; transport's claim that desktop writes after the flip are guarded rests on the first.
- Quarantine and bulk caps differ. Quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`), so its 5,000-FileId plan and 1 MiB body are gone; the selection caps are stated once in `WEB_CLIENT.md` (Limits). The reserved DELETE /api/items/{file_id} and POST /api/items/{file_id}/move in transport are retired, as writes does.
- Route names differ on the same operations. CLI cancel: transport DELETE /api/jobs/{job}; writes POST /api/jobs/{job}/cancel. Stop: transport POST /api/server/stop (build-exempt); writes POST /api/stop (no exemption, so a stale build cannot be stopped). Library roots and scans: transport /api/library/roots and /api/library/scans; writes /api/folders and /api/jobs. Inbox viewing: transport PUT /api/inbox/viewing with {viewing}; writes PUT /api/inbox/viewing/{client}.
- Yandex folder is not reconciled. Transport and identity say owner-wide, with GET /api/yandex/save-folder. Writes stores yandex_folder per desktop row. Transport also requires a folder token from the server's listing, while writes has the desktop send a chosen path that the server normalises. Desktop and phone must use one rule.
- Held polls are counted per device, but the device identity is undefined in transport (remaining item 'Device identity'). Adopt identity's device id (32 lowercase hex, minted by the server) now, so the two-held-poll cap and the per-device budgets have a key.
- Attach has no server.lock probe. Writes and identity both probe server.lock first, so a stale launch or server file alone never attaches. Transport's attach rule must include that step.
- Turn form is incomplete before migration 10. Transport gives only the versioned form {delta, version, op}. Writes gives an at-most-once form with request_id before 10 and the versioned form from 10. Transport's resolution must state the pre-10 phase and the request_id name.
- Not verified or not checked here: the Qt wasm WebSocket has no ping (qwebsocket_wasm_p.cpp, taken from the validators, not read); ureq holding the 25 s long poll (the `ureq` package is in Cargo.lock; the hold is untested); axum, hyper, h2 and tokio are not in Cargo.lock, so the rapid-reset and stream-cap facts are unchecked; whether HTTP/2 negotiates under native-tls on Windows is unchecked; Chrome behaviour (cancellation, placeholder reuse, Clear-Site-Data) is untested. Verified by reading: sql.rs:17-21 (meta.seq), render lib.rs:14, thumbcache lib.rs:89-91 (version mismatch forgets marks), lib.rs:25-27 (catalog_id start-over, which already exists), grid.rs:41 (56-byte assert), inbox.rs:62 (POLL 30 s), inbox.rs:431 (set_viewing, process-wide), schema.rs:377 (NORMAL), reader.rs:92 (READERS thread-local), images.rs:275 (full_image applies unwritten turns), Qt qnetworkreplywasmimpl.cpp:259 (LOAD_TO_MEMORY) and :276 (timeoutMSecs).

### identity

- Cookies conflict with transport and writes. Identity uses two cookies (`__Host-gallery_session`, SameSite=Strict; gallery_device, Path=/api/auth/login, SameSite=Strict). Transport uses one cookie, `__Host-pg_sign`, Lax. Writes uses one cookie, `__Host-gallery_session`, Lax. Identity's Strict rationale (the realsize argument) is not stated in the realsize cluster here, and writes' argument is that Strict breaks an external link. Identity's Lax-versus-Strict claim is the one that must change or be made an owner question.
- Store location conflicts with writes. Identity puts owner, devices, sign_ins and login_failures in <catalog_dir>/server.db and rejects auth.sqlite (auth design:159). Writes puts devices, logins, sessions and the verifier in auth.sqlite under pg-auth, and says server.db holds only remembered, request_ledger and job_reports. The settings table (identity) and remembered table (writes) are two names for the same store.
- Session listing and last_file are placed differently. Identity: catalog, rebuild-carried (rebuild.rs:148-149), Q10. Writes: remembered table in server.db with a version. Transport does not place them. One home must be chosen; the catalog copy is needed only as a rollback mirror.
- yandex_folder scope conflicts. Identity: owner-wide, settings row. Writes: device/desktop remembered row. Transport: owner-wide (GET /api/yandex/save-folder). Two of three agree, but identity records owner-wide only as a recommendation (Q6).
- Desktop principal key conflicts with writes. Identity rejects a fixed 'desktop' key and uses the random this_computer id. Writes section 3.1 keys desktop preferences by scope_kind device and scope_id 'desktop'. Writes must use the this_computer id, or identity must accept the fixed key with a reason.
- Launch file and handshake conflict with writes and transport. Identity: launch.json holds port and token, and GET /api/hello returns {build_id, catalog_id, proof}. Writes: server.token and server.json {port, envelope, build_id}, and GET /api/handshake. Transport: server.token and server.json, build_id only. Identity's hello omits the protocol envelope, so it contradicts the decided item at WEB_CLIENT.md:260 (the handshake carries a protocol version and a build id; the desktop refuses a different protocol version). Identity's text says 'the protocol integer is not returned (design:1237)' and must be corrected.
- Start-up takes an exclusive hold on server.db at every start (section 2.3, step 3: 'Take Live::exclusive on server.db. Judge the file'). Identity's own rule says no process takes Live::exclusive on server.db except to replace the file. The two statements contradict; the judge needs a read, so the exclusive is only for replacement.
- Thumbnail miss queuing conflicts with transport. Identity section 'Thumbnail GET and the no-state-change rule' says a miss enqueues cache work on an authenticated GET and that cache warming is allowed. Transport says the GET never queues (ops rule 15 has no exception) and only POST /api/thumbs/wants queues. The identity text must change to transport's rule.
- Pre-login set conflicts with writes and transport. Identity keeps the handshake out of the pre-login set; writes and transport keep GET /api/handshake in it. Identity's position is right for the LAN (WEB_CLIENT.md:274-275); the loopback handshake is outside the LAN gate and must be stated so.
- Cache-Control conflicts with transport. Identity: region and unversioned images are private, no-cache. Transport drops the no-cache rule for regions because the query names the region. Identity's Q4 recommends private, 31536000, immutable only for versioned URLs; transport's Q2 recommends 604800. The recommendation must be one value.
- CSRF header conflicts. Identity: X-Gallery-Request: 1 on every unsafe method. Transport: X-Pg-Client. Writes: X-Pg-Bundle. The writes cluster retires X-Gallery-Request, so identity must be rewritten to the chosen header.
- Logout Clear-Site-Data: identity sends cache and storage only, never cookies, because the device cookie must survive. This is correct only under identity's two-cookie design; transport sends cookies. The two cannot both hold.
- Write-back: identity elevates enabling only (Q1 recommended), stores it in server.db settings, and says it is server-wide. Transport stores it in gallery.json, file-first, with a lock. Writes stores it in the server remembered row. Writes Q-WB recommends elevation in both directions. The option sets differ.
- Preference failure semantics differ. Identity: a failed write answers 503 and is not held in memory. Writes section 9.4: NotPersisted, and the value is kept in memory. Transport: 507, memory unchanged (on a store.rs misreading; see transport problems). Identity must match the one answer.
- The offline credential refusal under WSL, and the refusal of a Direct-backend credential write (Q12), are refusals the decided processes rule does not contain (WEB_CLIENT.md:280-282: 'No refusal is needed'). Identity keeps the WSL refusal in its resolutions. It must be an owner question, and the resolution text must not present it as decided.
- Logon start: identity says the desktop spawn uses die_with_parent (crates/platform/src/process.rs:23-25 spawn_background, die_with_parent at :104), and the server must not be spawned that way. Writes and cutover's detached Command should replace this; identity's 'per-user background process' must name the spawn it uses.
- Owner question Q1 (write_back, enable-only elevated) differs from writes Q-WB (elevated both ways, or yes unspecified). Make the options identical in both clusters.
- Not verified: Layout::for_sqlite (identity cites layout.rs:87; the library's layout.rs was read at its catalog_dir and log lines only), remove.rs:21 move_aside, file.rs:184-188 (beside), file.rs:84-109 (TempFile), judge.rs:113-135 verdict lines (judge.rs:107 and :125 were confirmed as the judge function and judge_conn), and the argon2, getrandom and constant_time_eq signatures (not in Cargo.lock as direct deps; not checked). Verified by reading: lock.rs:12-13, 27-33 (Lock::take waits 100 tries at 50 ms), lock.rs:55 (Live::shared), lock.rs:70 (Live::exclusive), inbox.rs:62 (POLL 30 s), platform process.rs:23-25 (spawn_background calls die_with_parent).

### writes

- Thumbnail miss queues on the GET. Section 'Thumbnails for held rows' and the cutover paragraph say a miss returns a placeholder and queues the make keyed by (file_id, height). Transport decides the image GET never queues and only POST /api/thumbs/wants queues. This contradicts transport and identity's rule, and the events ops rule 15. The writes text must adopt the want-list queue.
- Placeholder body conflicts with transport. Writes says 'returns the ThumbHash placeholder'. Transport decides 200 image/png 1x1 transparent, no-store, X-Thumb-State pending, with the thumbhash drawn in CSS.
- Thumbs scope bump granularity differs. Writes says the thumbs scope bumps 'for that file at that height' and the desktop re-requests its placeholder cells. Transport bumps thumbs on each fill after commit, batched at 250 ms, and the client re-sends its whole want list. The writes wording must be transport's.
- CSRF rationale depends on a claim the writes cluster itself breaks. Writes says the cookie can be SameSite=Lax because 'GET changes no state (route-table test)'. Writes also queues the thumbnail make on a GET miss, which changes server state. Once the miss queue moves to the want list (as above) the rationale holds; until then it is wrong.
- CSRF header: writes uses X-Pg-Bundle as the non-simple header and rejects X-Pg-Client as the device design's placeholder. Transport requires X-Pg-Client on every unsafe method (the caller marker needed for the login POST, which is not build-gated). Identity uses X-Gallery-Request. Writes must adopt one name.
- Cookie: writes says `__Host-gallery_session`, SameSite=Lax, one cookie. Identity says two cookies, Strict. Transport says `__Host-pg_sign`, Lax, one cookie. Writes says 'the auth cluster must conform' with no argument, and the device-record lookup by the cookie is not stated.
- Device and login records location: writes puts them in auth.sqlite (pg-auth) and says server.db holds only remembered, request_ledger and job_reports. Identity puts owner, devices, sign_ins and login_failures in server.db and rejects auth.sqlite. Writes must state its position as contested, not as settled, or adopt identity's.
- Fixed desktop key: writes section 3.1 keys the desktop's remembered rows as device/desktop. Identity requires the random this_computer device id. Writes must change.
- Session listing location: writes puts it in server.db remembered (user/owner/listing). Identity puts it in the catalog (rebuild-carried, Q10). The writes text must say that the catalog copy is a rollback copy and not the authority, which is what section 3.3 already does, and identity must match.
- yandex_folder is in the device/desktop scope in writes; identity and transport say owner-wide. Writes must move it or make it an owner question.
- Migration 10 timing contradicts transport. Writes Q-T2 recommends the release after the cutover (B). Transport's owner question 8 recommends shipping 10 with the cutover, and transport's post-flip guards depend on that. The decided versioned-set rule is not met for the cutover release under writes (ratings and marks unconditional until 10), so section 'Decided items amended or confirmed' calling ratings and marks 'conforming' is wrong and must say 'amended until migration 10'.
- Migration 10 contents differ from transport. Writes: AUTOINCREMENT rebuild plus rating_seq, mark_seq and turn_seq. Transport: guard columns, files.changed extension, catalog ops table, job tables and AUTOINCREMENT. Job tables and the catalog ops table are absent from writes' migration 10, and job_reports is in server.db in writes but in the catalog in transport.
- Prepare lock 'fails at once' is wrong. Section 2.1 says a CLI prepare meanwhile fails at once with 'another process is preparing it', citing lifecycle.rs:43-47. Lifecycle take_lock calls pg_dbfile::Lock::take (lifecycle.rs:43-46; lock.rs:27-33), which waits up to LOCK_TRIES (100) at LOCK_WAIT (50 ms), about 5 s, before it fails. The text must say it waits about five seconds and then refuses.
- Stop route is blocked by the build gate. Writes section 2.6 step 5 requires X-Pg-Build on every route, so the desktop's POST /api/stop against an older build gets 409 build-changed before any handler. Section 2.3 relies on that route. Transport exempts POST /api/server/stop from the build gate; writes must do the same and must use the same route name.
- Handshake in pre-login: writes keeps GET /api/handshake public on the LAN, which breaks the decided item that login comes before any request other than login (WEB_CLIENT.md:274-275). The handshake is loopback-only for the desktop; writes must say so.
- Attach trust: writes reads the bearer from server.token and then calls GET /api/handshake, with no proof that the answering process is ours. A squatter on a stale port returns envelope 1 and the matching build_id, and the desktop then sends the bearer. Identity's keyed proof closes this; writes must adopt it or state the order (no bearer before the proof).
- Quarantine plan cap and body cap: moot, since quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`). The selection cap and body cap are one set, stated once in `WEB_CLIENT.md` (Limits).
- Route names differ from transport: writes POST /api/stop vs transport POST /api/server/stop; writes POST /api/jobs/{job}/cancel vs transport DELETE /api/jobs/{job}; writes PUT /api/inbox/viewing/{client} vs transport PUT /api/inbox/viewing; writes POST /api/folders vs transport POST /api/library/roots.
- Yandex save: writes has the desktop send the chosen folder path, normalised by pg_platform::names::normalize_path, and records it as a device/desktop row. Transport requires a folder token issued from the server's own listing. The rule must be one, and the desktop's native picker must have a stated route to a token.
- Q-CLI recommends refusing a CLI write while an in-process desktop holds server.lock. The decided processes rule says CLI writes go to the server while it runs and otherwise write directly, with no refusal (WEB_CLIENT.md:280-282). Writes must state the refusal as an owner question with the decided text, not as the recommended behaviour.
- Q-WB: the option 'Yes' does not say whether enabling only or both directions. Identity's Q1 recommends enable-only. Make the options identical in both clusters.
- Failed-write semantics: writes 4.3 says 503 not-persisted and the client keeps the value; 9.4 says NotPersisted keeps the value in memory. Transport says 507 with memory unchanged; identity says 503 and not held. One answer is needed.
- Not verified: the writes' store.rs:68-75 'temp removed on failure' claim (write_atomically is at store.rs:65-70 and was read only for the sync and rename order); prefs.rs:147-156; rebuild.rs:181-210 (copy_tables, not read); the cited schema.rs:502 statement (verified: no AUTOINCREMENT); lifecycle.rs:43-46 and core.rs:92 (verified); queue.rs:103-127 and events cites beyond these (not read in this pass). Verified: core.rs:92 takes Live::exclusive unconditionally, held through mark_unheld_missing at :115 and dropped at :123, with the refusal at :93-95 applying to changes only; core.rs:75-80 refuses a newer schema; core.rs:124 is the shared hold; schema.rs:377 synchronous NORMAL; schema.rs:43-44 migrations after TO_9; the SQLite claim is confirmed in the bundled sqlite3.c (aMJNeeded has WAL = 0, so a WAL database contributes no super-journal count).

### listing

- Header name conflicts with lan. Sections 2.2 and 2.8 require X-Gallery-Request: 1 on every POST. Lan 5.3 replaces that name with X-Gallery-Build, the one header that serves as CSRF guard and build check. Section 4.6 ('Transport') fetches image bytes with fetch() carrying X-Pg-Bundle to avoid a 426. Lan withdraws X-Pg-Bundle and exempts media GETs from the build header, so the 426 path does not exist.
- View cache policy conflicts with lan 10.4 and auth:247. Section 4.6 makes fit, tile and region 200s 'private, max-age=31536000, immutable' because each URL carries the full key. The key includes mtime and rotation, which cannot detect a same-mtime, same-size replacement (wire's own open item, design:1464; lan 10.4). Lan sets view images to no-store. The cluster states both positions without an owner decision.
- Sign-in cookie name is left open, but lan 6.1 resolves it to `__Host-gallery_session` with SameSite=Lax (auth:179 has the name with Strict; device:334 has `__Host-pg_sign`). The cluster also cites doc:155 for Lax; the Lax line is WEB_CLIENT.md:153.
- Cutover real_size import is not atomic as written. The row is the this_computer device row in server.db (lan 6.2 and section 9), but the marker is 'in meta', which is a catalog.sqlite table (crates/catalog/src/schema.rs:697-703). Lan 9 says no transaction spans the two databases.
- Stamp coverage (2.7 a and b) misses tables the replica and sorts read. Not stamped: ratings.rating (grid.rs:42 join, rating read at grid.rs:27; ratings table schema.rs:647-651, no trigger); file_writes.value, which is read as pending_turns and changes shown_size (grid.rs:43 and 69; schema.rs:664); border_marks presence (grid.rs:46, entry column 15; schema.rs:607-610); and analysis.error, which the join filters on (grid.rs:41 'a.error IS NULL'; trigger (b) omits error). The section 6 stamp test would fail on these.
- The rebuild as written drops triggers. DROP TABLE files removes every trigger on files: files_added (schema.rs:707), files_changed (719), files_source_changed (728), files_failures_stale (136 and 278), files_failures_hidden (283). Step 2.7(f) recreates only indexes at schema.rs:528 and 535-543, and misses files_folder and files_ref (530, 532) and files_fresh (351, from TO_9). Missed triggers stop stamps and failure rows silently.
- Rebuild transaction shape cannot be built as written. Step 2.7(f) turns foreign_keys off outside the transaction, but SETTINGS turns it on (schema.rs:377), prepare applies SETTINGS before bring_to (core.rs:101 and 107), and each migration runs in one transaction (schema.rs:23-26). PRAGMA foreign_keys is a no-op inside a transaction, and Step::Rust (schema.rs:19-22) also runs inside it.
- listing_key is build-scoped and the design does not say so. It is BLAKE3 over serde_json of the parsed Listing (view.rs:11-19; serde_json's float_roundtrip feature in `[workspace.dependencies]`). Any change to the Listing serialised form changes every key on upgrade, so a replica held across an upgrade is orphaned, not repaired.
- Poll description is incomplete against lan. Section 2.8 gives the 250 ms window but not the 25 s hold, the cap of four held polls per device, or the 429 (lan 8). The two clusters must state the same poll contract.
- Citation errors. schema.rs:514 is the kind column; the seq comment is schema.rs:523 and changed is 524. The index recreate range is 530-543, not 528 and 535-543. The doc cites are: SameSite Lax at WEB_CLIENT.md:153 (not 155); the 'not a new behaviour' rating claim at doc:222-224 (not 224-226).

### lan

- TLS premise is false. The TLS-stack resolution says 'ureq already uses ring, so one provider serves both'. The `ureq` entry in `[workspace.dependencies]` has features native-tls and cookies, and WEB_CLIENT.md:384 says ureq goes through native-tls (the `native-tls` package in Cargo.lock). The rustls change moves ureq off native-tls; it does not reuse an existing provider.
- Logon trigger default conflicts with a decided item. The decided text is 'an always-running per-user background process, started at logon' (WEB_CLIENT.md:266-267). Q1 options (b) and (c) contradict it, and the recommendation (b) is presented as satisfying the decided login-while-closed rule. Only (a) matches the decided text.
- Post-cutover CLI writes can block the server. Section 2.3 holds server.lock shared for a whole CLI write, and says a server start during it exits 75 and 'this delay is accepted'. After cutover the desktop has no catalog and needs the server for every read (cutover; section 2.4), and the server is the sole data authority (WEB_CLIENT.md:29-30). A long CLI scan would then leave the desktop without reads for its whole length.
- 'Ledger' collides with existing code. The catalog already has a ledger module for failure ledgers (crates/catalog/src/ledger.rs:1-11) and a ledger_ack table (schema.rs:125, 256), carried by rebuild (rebuild.rs:152). The new operation ledger is also called 'ledger'.
- The exclusive-probe scope is wider than the cluster states. Section 2.7 says the exclusive probe is taken only in a branch that needs a layout change. Core computes changes = why.is_some() || version != current (core.rs:91), so any migration (bring_to, core.rs:107), not only a layout change, takes the exclusive hold (core.rs:92-95). The decided rule covers layout changes only.
- Lock primitives named wrongly. Section 2.1 says Lock::try_take and Lock::try_take_shared sit beside Lock::take. In crates/dbfile/src/lock.rs, Lock is exclusive-only and waits (lines 19-38); the shared primitive is Live::shared (lines 41-61), on a .live file, not a .lock file.
- View cache conflicts with the listing. Section 10.4 says view images are no-store. The listing (4.6) makes fit, tile and region immutable. Auth:247 and the thumbnail rule differ too. The cluster records the thumbnail decision (Q3) but not the view decision.
- Change poll is still GET in this cluster. Sections 5.4 and 8 say GET /api/changes carries the vector. The listing and the decided header rule require POST /api/changes with X-Gallery-Build and Origin (the 25 s hold stays).
- Route table is incomplete. The one-table rule (section 5.4) names only this cluster's routes. The listing's routes (POST /api/listings/full, /delta, /facets; POST /api/records; POST /api/selections/apply) and PUT /api/settings/real_size, plus GET /api/settings, are not in the table, so the route-table test cannot cover them.
- Two servers under one home is not specified. Section 1 says a second catalog under one home is allowed. server.db is per home, so two servers share the device and sign-in tables, and only the LAN port prevents a second LAN listener. The rule for two servers (refuse, or allow) is missing.
- Doc citations for the WebSocket Origin rule are wrong. Section 13 cites WEB_CLIENT.md:403-404 and 412-413. Line 403-404 is the 'Behaviour the server must have' heading, and 412-413 is 'Open decisions'. The WebSocket Origin text is at 409-410, and the endpoint at 400-401.
- Cookie Max-Age differs from the device design. Section 6.1 re-issues the cookie once a day, but device:334 says last-used is written at most hourly. The change must be stated as a change to device:334.

## Decided items that the designs conflict with

- (transport) Decided: the CLI's writing commands go through the server's API while it runs; no refusal is needed.
- (transport) Decided: the exclusive hold on the catalog is taken only for layout changes; ordinary SQLite writes share the catalog.
- (transport) Decided: mutations are versioned sets (WEB_CLIENT.md:27-28; State model 229-233, which says relative operations are computed by the client into absolute values).
- (transport) Decided: a cache miss queues the work and returns a placeholder (WEB_CLIENT.md:410-411).
- (transport) Decided: login comes before any request other than login (WEB_CLIENT.md:274-275).
- (transport) Doc: the events socket and its Origin check (WEB_CLIENT.md:273, 412-413, 499-549); the status section records live changes as decided but not the socket.
- (transport) Decided: listings are stamped (listing_id, generation) (WEB_CLIENT.md:26-28, 205-206).
- (transport) Decided: write_back is server-wide and owner-changeable from any client, on by default (Status lines 30-31).
- (transport) Decided: video is open and not designed.
- (transport) Decided: the server is the sole writer of the catalog after the desktop cutover.
- (identity) The exclusive hold on the catalog is taken only for layout changes; ordinary SQLite writes share the catalog.
- (identity) write_back is server-wide, changeable from any client by the owner, on by default.
- (identity) Login works while the desktop is closed (WEB_CLIENT.md:276-278).
- (identity) Photos always open fitted; no real_size setting for the web client.
- (identity) The server is the sole writer of the catalog; the desktop talks to it over loopback.
- (identity) WEB_CLIENT.md:193 and P1: yandex_folder per device.
- (identity) Login before any request other than login (WEB_CLIENT.md:274-275).
- (identity) The CLI's writing commands go through the server while it runs; no refusal is needed.
- (identity) Owner-level questions must not be decided by the designs.
- (identity) The desktop cutover moves its reads and writes together, in one step.
- (identity) Events SameSite and login origin (events design:1309, 1228-1230).
- (writes) Every mutation is a versioned set set(field, value, expected_version); relative operations are computed by the client into absolute values
- (writes) The handshake carries a protocol version and a build id; the desktop refuses a server with a different protocol version
- (writes) The exclusive hold on the catalog is taken only for layout changes; ordinary SQLite writes share the catalog
- (writes) Login works while the desktop is closed; the logon task is a separate opt-in (WEB_CLIENT.md:276-277)
- (writes) Only the server writes the catalog (WEB_CLIENT.md:26-27) and CLI writes go through the server while it runs (WEB_CLIENT.md:280-282)
- (writes) Moves are in version 1 once quarantine exists (decided: deletes and moves are in version 1). Quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`); whether a move is in version 1 is open (O40).
- (listing) WEB_CLIENT.md Status and State model: listings are stamped (listing_id, generation) and every listing message carries it (doc:26-28, 205-206)
- (listing) Decided: real_size is not a setting for the web client (doc:193 and the State model Preferences sentence)
- (listing) WEB_CLIENT.md:211-227 snapshot lifetime (grace, 10-minute idle, cap of four, release on disconnect)
- (listing) WEB_CLIENT.md:224-226: a rating reorder is 'not a new behaviour'
- (listing) Decided: scan-driven changes reach clients live
- (listing) Device design (device:342): real_size is a prefs field for every device kind, with browser defaults
- (listing) Auth design (auth:247): view endpoints are private, no-store
- (listing) Decided: the exclusive hold is taken only for layout changes (doc:280-282)
- (listing) Wire design (design:1285): the change poll is GET /api/changes
- (lan) The exclusive hold on the catalog is taken only for layout changes; ordinary SQLite writes share the catalog (decided, Processes and access).
- (lan) While the server runs, the CLI's writing commands go through the server's API; no refusal is needed (decided, Processes and access).
- (lan) The server is the sole data authority and the only writer of the catalog; after the one-step cutover the desktop writes nothing (decided).
- (lan) Login before any request other than login (decided; WEB_CLIENT.md:272-273).
- (lan) The login works while the desktop is closed (decided, Processes and access).
- (lan) Scans and sync run in the background at low I/O priority (decided, WEB_CLIENT.md:268).
- (lan) A private CA is already configured and the phone trust test is the gate (WEB_CLIENT.md:272).
- (lan) The native-tls facts cited as reasons against it (WEB_CLIENT.md:380-387).
- (lan) A cache miss queues the work and returns a placeholder (decided), together with the rule that no state-changing route answers GET (ops, wire).

---

## Web and phone client: reconciled design

## How to read this

This document reconciles the five judged clusters (transport, identity, writes, listing, lan) into one design. Each cluster section states the reconciled design, then the contradictions it resolved and what was rejected, then what could not be verified. Validator corrections are applied where they fix the text. Questions that belong to the owner are listed in full under Owner decisions and are not decided here; the recommendation shown is advice only. Where the assembler chose between cluster positions to remove a contradiction the validators flagged, the text says ASSEMBLER. Byte counts are arithmetic on field widths. Nothing was built, run or measured. Items marked (P) are proposals to measure before they are fixed. Citations are file:line as the clusters give them.

Names used throughout (every section uses these spellings):

- Cookies: `__Host-gallery_session` (the sign-in secret; Secure, HttpOnly, SameSite=Lax, Path=/, no Domain) and `gallery_device` (the device id, which grants nothing; Secure, HttpOnly, SameSite=Lax, Path=/api/auth/login). See 2.4.
- Headers: `X-Pg-Client: browser|desktop|cli` on every POST, PUT, PATCH and DELETE; `X-Pg-Bundle` (browser) or `X-Pg-Build` (desktop, cli) on build-gated requests; `Authorization: Bearer` on loopback; `Origin`; `Sec-Fetch-Site`.
- Routes: `/login` and `/api/auth/*` for sign-in; `/api/devices/*`; `/api/settings/*`; `/api/hello` and `/api/server/stop` (loopback only).
- Files in catalog_dir: catalog.sqlite, server.lock. Files in home: server.db, launch.json, gallery.json (rollback mirror), gallery.log, desktop.lock, home.lock (ASSEMBLER), tls/lan-ca.bundle, tls/root.cer.
- Ids: 32 lowercase hex, 128 random bits, minted by the server: device id, request_id, plan, job, sid, epoch. listing_key is derived (section 4). catalog_id is meta.catalog_id.
- Errors: body is `{error: <kebab-case code>}`.

## Owner decisions

Each line gives the question, the options, the recommendation, and the sources it merges. Duplicates across clusters are merged once.

Access and devices

- O1. Does changing write_back from a phone, or revoking another device, need the owner password re-entered? Options: (a) both, and bulk forms; (b) write_back enabling only, single revoke without password, bulk revoke with password; (c) neither. Recommendation: (b). Enabling write_back lets the server write into originals; a single revoke is the lost-phone action and must work without a password. Sources: identity Q1-Q2, lan Q16, writes Q-WB, transport decided-conflict list. Lan and writes recommend (a) for write_back.
- O2. Session lifetimes. Idle is 30 days (agreed). Absolute: 90 days or 180 days. Recommendation: 90 days. Sources: transport 16, identity Q3, lan Q2. Accepted at 90 days (`WEB_CLIENT_TECH_DECISIONS.md:146`, 2026-11-14); the 180-day device-design value is void.
- O3. Password minimum (15 characters, agreed in recommendations) and Argon2id cost (proposed m=64 MiB, t=3, p=1). Options: fix now; or choose the memory cost by measuring one verification on the owner's desktop at about 1 s, not above 64 MiB; or the OWASP floor of 19 MiB. Recommendation: 15 characters, then measure. Sources: identity Q5, lan Q9. Decided by default in this pass: Argon2id 128 MiB, t=3, p=1, the cost accepted in `WEB_CLIENT_TECH_DECISIONS.md:147` (0.59 s measured there); the owner may revisit.
- O4. Revoke and the password: (a) accept and document; (b) revoke always changes the password; (c) a choice at revoke time. Recommendation: (c). Source: identity Q9.
- O5. Logon start. The decided text is an always-running per-user background process started at logon (WEB_CLIENT.md:266-267). Options: (a) logon trigger always on (matches the decided text); (b) trigger only while the LAN is enabled (lan's recommendation; amends the decided text); (c) off. Restart policy: 3 restarts at 1 minute, or none. Recommendation: (a) with 3 restarts. Sources: writes Q-LOGON, identity Q8, lan Q1.
- O6. The WSL refusal for the offline credential command (identity). Options: keep the refusal; allow. Recommendation: keep, and state it as an exception to the decided no-refusal text.
- O7. Quarantine purge over the LAN in v1. Options: disabled, loopback only, typed confirmation above 100 items; or elevated over the LAN (confirmation above 500). Recommendation: disabled. Sources: identity Q11, writes Q-PURGE. Moot: quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`), so there is no purge command anywhere.
- O8. Session listing and last_file: per user (follows the owner, version-checked) or per device. Recommendation: per user. Sources: transport 10, identity Q10, writes Q-SCOPE.
- O9. Remembered listing save timing: at once (recommended, 250 ms floor) or debounced by 1 s. Source: transport 11.
- O10. Accept up to 1 s of last-file and session changes lost when Android kills the page with no event (keepalive flush on hidden and pagehide). Recommendation: accept. Source: writes Q-KILL.
- O11. Cache-Control for keyed image responses (thumbnails, fitted views, tiles, regions, Yandex images). Options: (a) private, max-age=604800, immutable; (b) private, max-age=31536000, immutable (transport's working value); (c) private, no-cache with ETag (lan Q3); (d) public (rejected: a shared cache could hold authenticated bytes). Recommendation: (a). The key includes mtime, which cannot detect a same-mtime, same-size replacement; the catalog already has that limit. Sources: transport 2, identity Q4, lan Q3, listing 4.6. Decided by default in this pass: option (a), private, max-age=604800, immutable (`WEB_CLIENT.md`, `WEB_CLIENT_DESIGN.md`); the owner may change it.
- O12. Thumbnail miss answer. Working value: 307 to the nearest cached height in S, or a 1x1 placeholder; work queued only by the want list. Alternative: 200 nearest body with an X-Thumb header (wire). Recommendation: the working value; confirm the 307 and the placeholder on Chrome Android. Sources: transport 3, writes, identity.
- O13. Desktop thumbnail heights: quantised to S for every client (recommended) or an unquantised desktop cache (a second cache writer). Sources: transport 4, writes Q-THUMBH.
- O14. Turn form: (a) relative delta with turn_seq guard from migration 10 (recommended); (b) relative delta with request_id and no guard (the only form before migration 10); (c) absolute set with expected_version. Sources: transport 5, writes Q-T1, listing (viewstate).
- O15. Ratings and border marks carry their version from migration 10. Options: versioned (recommended); last writer wins. Source: transport 6.

Catalog, schema and cutover

- O16. Listing model. (A) client-held replica: stamp becomes (listing_key, seq), no server listing state, no cap or budget (listing cluster's recommendation). (B) server-held frozen membership: the decided stamp (listing_id, generation) stands, cap of 4 per device (transport and identity working value). Recommendation: A, which changes a decided item and needs approval; section 4 gives both. Sources: transport 7, identity Q13, listing 1, writes Q-LIST, lan Q17.
- O17. Timing of migration 10 (AUTOINCREMENT rebuild, seq columns, stamps, tombstones, ops and job tables). Options: (a) with the cutover release (transport); (b) the release before cutover (listing Q7); (c) a release after cutover, before the first phone mutation (writes Q-T2; lan Q14). Recommendation: (c). The cutover release stays schema-neutral, the FileId reuse exposure exists today (pair.rs:345, 434), and the cost is stated: desktop writes stay unversioned until that release.
- O18. Binary rollback once migration 10 has shipped (a pre-migration build refuses the catalog, core.rs:75-80). Options: forward fix only; restore the pre-migration backup, losing later writes; or both. Recommendation: both, with the backup as last resort. Source: writes Q-T3.
- O19. An unmodified pre-cutover build run beside a running server is not stopped by any lock. Options: accept and document (the rollback procedure stops the server first); or amend the decided rule with a momentary exclusive probe at server start only. Recommendation: accept and document. Source: writes Q-OLD.
- O20. A CLI write while an in-process desktop holds server.lock shared (before the flip). Options: allow (ordinary writes share the catalog; SQLite serialises them); refuse with a message. Recommendation: allow. Sources: transport 13, lan Q10 (allow); identity Q12 and writes Q-CLI (refuse). The recommendations differ; the owner decides.
- O21. ASSEMBLER-ADDED (from the lan validator). A server start while a CLI writing command holds server.lock shared. Options: (1) the start waits up to a bound and the desktop shows a notice; (2) the CLI yields at a file boundary when a start is pending; (3) accept that the start exits and the task retries. Recommendation: (1). Without a choice, a long CLI scan leaves the desktop without reads.
- O22. A second in-process desktop process while the first runs. Options: refuse (desktop.lock) or allow. Recommendation: refuse. Source: writes Q-DESK2.
- O23. In-band Direct desktop backend (GALLERY_BACKEND=direct) kept for one release as a rollback lever. Options: keep for one release, then remove; or remove at the flip. Recommendation: keep for one release. Sources: transport 14, writes Q-DIRECT, lan Q11.
- O24. Does the scanner keep a rating when a file is replaced (pair.rs:10, scanner decision 7)? Recommendation: keep; the scanner's owner confirms. Source: transport 15.
- O25. Ledger placement (lan Q13). Options: catalog ops table in catalog.sqlite for catalog effects plus a server request_ledger (recommended); or the at-most-once turn fallback only.
- O26. Restate the Status line (WEB_CLIENT.md:26-27) as: the server is the only writer while it runs; direct writes happen only while holding server.lock with no server running. Options: confirm (recommended); or keep the line and refuse CLI writes when no server runs. Source: writes Q-STATUS.
- O27. Remove the WebSocket: accept the change-vector long poll in its place (WEB_CLIENT.md:273, 412-413, 499-549 change). Recommendation: accept; Qt wasm has no WebSocket ping. Sources: transport 1, lan Q15.
- O28. Loopback stays plain HTTP on 127.0.0.1 with the bearer token and no TLS. Options: confirm (recommended); require TLS. Source: lan Q12.

Desktop and phone behaviour

- O29. Desktop real_size and the Z mode. Options: keep both on the desktop in v1 (recommended by writes and listing); remove them so the desktop opens fitted as the web does. The web client has neither setting. Recommendation: keep. Identity Q7 recommended removal. Sources: transport 9, identity Q7, writes Q-REAL, listing Q5.
- O30. Yandex saves from a phone in v1. Options: (a) phone saves into the owner's configured folder, named only by a folder token (transport's recommendation); (b) no phone saves in v1 (writes and listing recommendation); (c) any library folder (rejected). Recommendation: (b) until a phone folder rule is designed. Sources: transport 12, writes Q-YANDEX.
- O31. Yandex result folder: owner-wide (recommended: start and save act on the server host, yandex.rs:520-527, 533-550) or per device (rejected). Sources: identity Q6, transport decided-item conflict.
- O32. Yandex 1:1 on the web. Options: hidden in v1 (recommended); host-URL source; server proxy of result images. Source: listing Q8.
- O33. Yandex revocation change: online end-certificate checks, soft-fail on unknown status, connect-time wait up to about 10 s. Options: accept, gated by a paired live measurement (recommended); or keep the schannel cache-only check on Windows. Source: lan Q4.
- O34. Bulk destructive action from a phone (select-all delete or move) needs password re-entry? Options: no, quarantine is the safety net (recommended); yes. Source: listing Q9. Moot for the safety net: quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`), so a bulk delete goes to the Recycle Bin; whether it needs re-entry is still open.
- O35. A Delta that moves the list while the viewer is open or the grid is scrolling: apply on viewer close or 300 ms after scrolling stops (recommended); apply at once; or only on reopen. Source: listing Q2.
- O36. When the selected file leaves the list: the next surviving FileId in the old order (recommended; a desktop change); the next in the new order; or the clamp at the old index (current desktop). Source: listing Q3.
- O37. Provisional rows (unpaired = 1) listed? Recommendation: not listed until pairing ends (a desktop change). Options: listed (current desktop). Source: listing Q4.
- O38. Restore the open viewer and modal panels after a reload. Recommendation: no in version 1. Source: writes Q-VIEWER.
- O39. May a Start override (--query, --sort, --size, --borders, --dev, --filters) reach the saved session? Recommendation: no; the current leak is at client controller.rs:218 and :533 and contradicts the comment at :34-36. Source: writes Q-OVR.
- O40. Version-1 moves. Options: quarantine and restore to home only (recommended); same-volume folder-picking moves (Q3) in version 1 only if stated. Cross-volume moves are refused in every case. Source: writes Q-MOVE. Moot for quarantine: it is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`); whether any version-1 move exists is still open.
- O41. Quarantine UI numbers (proposal, unmeasured): 30-day age offer that never runs purge; warning at 10 percent of the volume's held bytes; undo for 10 s until the next batch or restart; typed confirmation above 100 items. Options: as proposed (recommended) or measured values. Source: writes Q-QUI. Closed: quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`).
- O42. Sleep. Options: job-scoped keep-awake, with a permanent setting off by default (recommended); permanent setting on by default; no keep-awake. Source: lan Q8.
- O43. LAN default and network profile. Options: off by default; Private profile and LocalSubnet rule required (recommended); on by default; Public allowed with warning. Source: lan Q6.
- O44. Private CA structure. Options: three levels with the root key kept off the server (recommended); two levels with the root on the server (rejected). Source: lan Q7.
- O45. Qt licence route for the WASM bundle, and the repo LICENSE file. Options: LGPL-3.0-only route with relink obligations; Qt commercial licence; defer. Recommendation: defer until legal review. Source: lan Q5.
- O46. Write-back-related: Yandex and session items are O30-O32; the logon restart and task are O5.

## Decided items this design relies on

- The server is async (axum on tokio), the sole data authority and the sole catalog writer. The desktop talks to it over loopback with a per-start token (WEB_CLIENT.md Status).
- LAN access requires login. The phone holds an httpOnly cookie. There is no pairing and no QR code.
- Login comes before any request other than login (WEB_CLIENT.md:274-275). The pre-login set is GET /login and POST /api/auth/login only.
- The handshake carries a protocol version and a build id; the desktop refuses a different one (WEB_CLIENT.md:260). The protocol version is the envelope integer 1, returned only by the loopback hello.
- While the server runs, CLI writing commands go through its API; no refusal is needed (WEB_CLIENT.md:280-282). Before any server runs, CLI writes go direct.
- The exclusive hold on the catalog is taken only for layout changes; ordinary SQLite writes share the catalog.
- The desktop cutover moves reads and writes together, in one step. Afterwards the server is the only writer.
- Deletes go to the Windows Recycle Bin and are in version 1; whether a move is in version 1 is open (O40). Quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`, recorded in this pass on 2026-11-21), so there is no purge command and no held-file store.
- A thumbnail cache miss queues the work and returns a placeholder. The queue trigger is the want list (O12).
- write_back is server-wide, changeable by the owner from any client, and on by default.
- Scan-driven changes reach clients live. A phone can start scans and imports.
- Layout runs in a Web Worker. The phone target is Android Chrome only.
- Yandex is in version 1.
- Photos always open fitted. The web client has no real_size setting. The 1:1 button stays.
- Video is work in progress and is not designed here.
- Login works while the desktop is closed (WEB_CLIENT.md:276-278); this needs the logon task (O5).
- Mutations are versioned sets (WEB_CLIENT.md:27-28). Amended, as a recorded change: turns are guarded relative deltas (O14). Ratings and marks are versioned from migration 10, not in the cutover release (O15, O17).
- Listing stamp (listing_id, generation) (WEB_CLIENT.md:26-28): kept under O16 option B; replaced under option A only with owner approval.

---

## 1. Transport, change channel, thumbnails and the wire surface

### 1.1 Change channel

- One channel: `POST /api/changes`. Body `{epoch?, scopes: {scope: gen}, listings?: [{listing, seq, catalog_id}]}`. The `listings` field is used only under option A (O16). Fixed scopes: tree, items, thumbs, inbox, jobs, settings, yandex. Under option B the listing scopes `listing:L:membership` and `listing:L:shape` (L is the 32-hex listing id) join the map. The first request omits `epoch`.
- Answer 200 `{epoch, scopes: {...}, moved: [listing_key]}`. A held answer covers every named scope that still exists and every fixed scope.
- Answer at once if the epoch differs, a named scope's generation differs, or a listing entry is moved. Otherwise hold up to 25 s (P). A bump is held 250 ms more so that bursts batch. The client's transfer watchdog is 35 s.
- At most two held polls per device (the limit stated once in `WEB_CLIENT.md`, Limits). A third gets 429 rate-limited with Retry-After. Revocation, session expiry and stop answer that device's held polls with 401 (stop answers the current vector).
- Clocks. `meta.seq` (sql.rs:17-23) is a durable row stamp: it sets record versions and the `as_of` guard, and nothing streams or replays it. Each scope has an in-memory u64 generation, from 0 in each epoch. The epoch is 128 random bits drawn at each start and never persisted.
- Bumps happen after COMMIT, batched 250 ms; `jobs` coalesces to 1 Hz. `items` moves whenever a commit moves meta.seq (read inside the transaction before and after). `thumbs` moves after a fill's commit or a NotStored. `yandex` moves on any search change. `tree`, `inbox` and `settings` move on their writes. Listing scopes (option B) follow section 4.
- Client. It reads a scope's generation before its data and adopts it only after that scope's refetch succeeds; adoption is monotonic. A record applies only if its `changed` is greater than the one held. Refetch rounds are at least 250 ms apart. Backoff after a failed poll: full jitter, base 500 ms, factor 2, cap 30 s (P), reset on the first good answer. Visibility or online aborts a held poll and polls at once. A 401 stops polling and shows login.
- Echo: events carry no op id. The acting client applies its response and refetches the scopes it moved.
- Qt wasm loads whole response bodies (qnetworkreplywasmimpl.cpp:259), so poll answers stay a few hundred bytes.

Resolved in 1.1:

- The WebSocket with a bounded queue and resume (events D14, thumbs, wire), SSE, a thumbnail stream and interval polling are rejected. The vector is complete in one answer, so no server queue or replay exists.
- Poll is POST, not GET, because the listing entries carry a spec that must not sit in a URL (listing).
- Two held polls per device, not four (events proposal). Thumbnails use the six HTTP/1.1 connections per host, and one page keeps five for images.
- The request form `scopes=name:gen` repeated per scope is rejected (ambiguous with colons). Answering with the whole server vector is rejected (it leaks other devices' listing scopes).

### 1.2 Request gate

Every request passes these steps in order. A failed step ends the request before any handler, cache read or queue work.

1. Host. Loopback: `127.0.0.1:port` or `localhost:port`. LAN: the owner's stable name or the bound IPv4, with the port when it is not the default. Otherwise 421 misdirected with an empty body.
2. LAN peer. The IPv4 peer must lie in the adapter's prefix, or the connection is closed before TLS.
3. Unsafe methods (POST, PUT, PATCH, DELETE). Origin equal to the configured origin (loopback refuses any Origin); `Content-Type: application/json` on POST and PUT (415 otherwise); `X-Pg-Client` present. Failure: 403 origin-refused. This applies to the login POST.
4. Cross-site subresource. `Sec-Fetch-Site: cross-site` with a `Sec-Fetch-Mode` other than `navigate` gets 403 origin-refused.
5. Pre-login set: `GET /login` and `POST /api/auth/login`. These skip steps 6 and 7 only.
6. Credential: sign-in cookie (LAN) or Bearer (loopback); else 401 not-authenticated. A document request without a session is redirected to /login.
7. Device: not revoked and not expired; else 401.
8. Build gate for build-gated routes: the header for the caller's kind (`X-Pg-Bundle` for browser, `X-Pg-Build` for desktop and cli) must equal the server's. Mismatch: 409 build-changed with the frozen body, no handler runs. Absent: 426 build-required. Exempt: `GET /login`, the bundle files and the image GETs (they cannot send headers; their keys carry the versions), `GET /api/hello`, and `POST /api/server/stop`.
9. Route: the route's declared body cap, then its rate limit, then the handler.

Image GETs run steps 1, 2, 4, 6 and 7, then the route. A route-table test fails on any undeclared body cap and on any state-changing GET except the thumbnail route (section 1.5).

Resolved in 1.2:

- Lens B's explicit order is kept, since it states the peer and cross-site checks. Lens A's build gate and the login POST's Origin check are kept.
- Events' step 2 exemption of the login POST from Origin is rejected (decided D19).
- Lens A's frozen-hashed-bundle exemption from the credential step is rejected: it breaks the decided login-first rule.
- A build check on images is rejected: they cannot send headers.
- The pre-login set is GET /login and the login POST only. The notices file, the favicon 204, the events handshake GET and the hashed bundle files are removed from it (ops, events). The favicon is a data URI inside the login page.

### 1.3 Build identity, bundle identity and loopback

- `build_id`: BLAKE3 over every workspace crate's sources and manifests, Cargo.lock and the workspace manifest, computed by a build script with rerun-if-changed. QML is excluded (live_reload.rs:1-6), so a QML edit does not restart the server. ASSEMBLER: lan's definition (manifest plus crate version) is rejected, because a source change that keeps the version would not change it.
- `bundle_id`: BLAKE3 over the bundle manifest (names and hashes). index.html carries it and sits outside the digest.
- One process serves one build_id and one bundle_id for its life. Changes need a restart.
- Stale signal: 409 build-changed; a bundle_id mismatch on the login page; an undecodable response on a build-gated route. The page issues no new requests, waits up to 3 s for sets in flight (each gets an answer; a 409 means nothing was applied), then navigates to `index.html?b=<server bundle_id>&n=<hop>`. The hop limit is 3. No storage is used.
- Frozen set (envelope 1; golden fixtures in pg-wire): the hello reply, the login reply, and the bodies of 401 not-authenticated, 403 origin-refused, 409 build-changed, 426 build-required and 429 rate-limited. version-mismatch is not frozen, because the build gate answers a stale page first.
- Wire enums carry no serde `other` and no defaults on required fields; an unknown value is a decode error, which on a build-gated route is a stale signal.
- Loopback hello: `GET /api/hello?nonce=<64 hex>`, no bearer. Returns `{envelope: 1, build_id, catalog_id, state, proof}`, where `proof` is the hex BLAKE3 keyed hash, keyed with H = BLAKE3(token), over the 32 nonce bytes. Only H is held in server memory.
- Files: `launch.json` in home (B9 in WEB_CLIENT_TECH_DECISIONS.md), `{port, token, envelope, build_id, started_ms}`, with the token as 32 random bytes in hex, written at each start by temp-then-rename under an owner-only DACL (not verified). It is removed at clean stop and trusted only while server.lock is held.
- Loopback request rules: Host must be `127.0.0.1` or `localhost` with the port; any Origin is refused; cookies are never read; no CORS. Every route except hello requires the Bearer token, compared in constant time. The token never appears in a query string or a cookie.

### 1.4 Error codes and headers

- All codes are kebab-case (the codebase's serde convention: 11 kebab renames, none snake_case). Body: `{error: code}`.
- Set: bad-request; not-authenticated (401); origin-refused, forbidden-scope (403); not-found; build-changed (409); build-required (426); version-mismatch; selection-changed; set-changed; stale-key; gone (410); snapshot-expired (option B only); precondition-failed (412); selection-too-large (413); not-member; misdirected (421); not-applicable (422); refused (422, with a reason); bad-listing; rate-limited (429, Retry-After); busy (503, Retry-After); not-persisted (503); storage-full (507); plan-expired (410); request-reused (422); purge-lan-disabled (403); device-limit (409); not-revocable (403).
- Mapping: SQLITE_FULL rolls back and answers 507 storage-full, with no ops row. Any other failed commit answers 503 not-persisted; login and revoke add Retry-After. SQLITE_BUSY past the busy timeout answers 503 busy.
- Headers: `Cache-Control: no-store` on JSON, listings, auth responses; `Cross-Origin-Resource-Policy: same-origin` on images; `X-Thumb-State: pending` on placeholders; `ETag` on the tree and views; `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` on every LAN response; no HSTS; no CORS headers.

### 1.5 Thumbnails

URL: `GET /api/thumbs/{catalog_id}/t{THUMBS_VERSION}/{file_id}@{mtime}.{rotation}/{height}`. THUMBS_VERSION is the render constant (crates/render/src/lib.rs:14). No turns in the key (the client applies them in CSS). height must be in S, the server's served-height set (geometric, 96 to 1024 at about 1.25, P); outside S or 16..2048 is 400 bad-request. Unknown query parameters are ignored.

Responses (after gate steps 1, 2, 4, 6, 7; the request never reads an original, never waits and never queues):

| Case | Response |
|---|---|
| Exact key cached (or in the recent-fill map) | 200 image/jpeg; Cache-Control per O11 |
| Exact missing; a height in S cached for this file, mtime and rotation | 307 to that height's exact URL; no-store (lookup `get_nearest_into`, thumbcache lib.rs:138, restricted to S) |
| Nothing cached for this file | 200 image/png, a 1x1 transparent image; no-store; X-Thumb-State: pending |
| Key not current (catalog_id, THUMBS_VERSION, mtime, rotation) | 409 stale-key; no-store; the client re-reads the item |
| File missing | 404 not-found; no-store |

Only the exact 200 is immutable, so non-exact bytes never enter a browser cache under the requested URL. The X-Thumb header is retired. `get_exact_into` is new.

The want list: `POST /api/thumbs/wants {viewport: [paths], background: [paths]}`. Total at most 512, first entry most important, each path the exact URL path (about 91 bytes at most; 512 paths about 48 KB, arithmetic). Body cap 64 KiB. The list replaces the device's previous list. A malformed entry or a height outside S fails the whole list with 400. Answer 200 `{queued, cached: [paths], stale: [paths], over_cap}`. Cached paths let the client upgrade cells without a GET. Over the device or global cap, the key is counted in over_cap; no 503 is used on the image GET. A queued key absent from the device's latest list is dropped if not started; a started fill always completes. Disconnect or revocation deletes the device's list.

Queue and decode:

- One fill queue keyed by (catalog_id, file_id, mtime, rotation, THUMBS_VERSION, height); duplicates merge. The per-device and global queue caps are the limits stated once in `WEB_CLIENT.md` (Limits).
- Priority: lane first (viewport before background), then the best list position across devices, ties LIFO.
- One fixed decode pool of min(4, cores). Viewport entries are taken first. The background pass (thumbs lib.rs:173-179) and background entries run on the same workers. Lanes are priorities, not separate pools, so the decoder count stays bounded. Background CPU priority is open.
- Each fill re-checks the cache before it decodes.

Fill, store and ready:

- A fill waits for `put_batch` to commit (thumbcache lib.rs:198). After the commit, or after NotStored, the `thumbs` scope bumps (250 ms batch).
- The client re-sends its list on any thumbs bump while cells are pending. A list response's `cached` paths upgrade the cells showing a placeholder: clear src, reset to the URL.
- Race rule: the client records the thumbs generation at each send; if a response arrives after the generation advanced, it re-sends at once. Backoff 1 s to 30 s applies only when no bump arrives.
- Store failure: bytes stay in a recent-fill map (64 MB, 10 minutes), serving the exact key from memory until the store succeeds or the window ends. NotStored posts one notice. Viewport fills pause 30 s.
- Thumbnail writes are NORMAL durability (derived data).

Cache identity and maintenance:

- A cache opened for another catalog_id, or with another THUMBS_VERSION, starts over. This is a required code change: today a version mismatch only forgets checked marks (thumbcache lib.rs:89-91).
- Readers: a bounded pool of N read connections (4 to 8), checked out per read; web reads run in spawn_blocking under a semaphore of N. The thread-local READERS (reader.rs:92-108) is removed. close() drains the readers, then stops the writer (lib.rs:307-313).
- Eviction spares heights wanted in the last 10 minutes and the pass height, with `height NOT IN (spared)`. On-demand puts record their height (writer.rs:255-257 keeps none today).
- Desktop: heights quantised to S for every client (O13). The Loopback backend fetches bytes through the trait over loopback with the bearer and never writes the cache. The Direct backend keeps make-on-read until it is retired (a listed parity exception).

HTTP/2 (only where negotiated; section 5.12): SETTINGS_MAX_CONCURRENT_STREAMS 128; a connection resetting more than 100 streams in 10 s is closed (P). Both need hyper and h2 versions that carry the rapid-reset fixes, which are not in Cargo.lock.

Resolved in 1.5:

- The image GET never queues. Writes' and identity's GET-side queuing and lan's enqueue-on-GET exception are rejected. The decided "queue the work" rule holds through the want list.
- The placeholder is a 1x1 PNG with X-Thumb-State; writes' "ThumbHash placeholder" wording is rejected.
- One queue with lane priority. Lan's separate visible and batch queues, with a 60 s expiry, are rejected; the want list already supersedes stale entries.
- Lan's HTTP/2 values (32 streams, 200 resets) are rejected for these (P); the fast-scroll reset rate is measured.

### 1.6 Mutations

Routes (each carries request_id unless it is a set by value):

- `PUT /api/items/{file_id}/rating {value: null or 1..5, version, request_id}`. Null deletes the rating; 0 is 400.
- `PUT /api/items/{file_id}/mark {marked, version, request_id}`. Marking needs borders Found, checked in the transaction (grid.rs:57, 74-79); otherwise 422 not-applicable.
- `POST /api/items/{file_id}/turns {delta: 0..3, version, request_id}`. Additive mod 4 in the same transaction, guarded by turn_seq. Delta 0 is unchanged. Before migration 10 the body has no version (section 3.6).
- A missing file is 410 gone. Responses: 200 `{result: applied|unchanged, record, affects_listing}`, plus `write` (queued, not_queued, not_possible) on ratings.

Rule. Each mutation runs in one IMMEDIATE transaction on the mutation connection (FULL, section 5.13). Value equal: unchanged; no bump; the ops row is written. Version equal: apply; bump the guard and `changed`; write the ops row. Otherwise: 409 version-mismatch with the current record, and a conflict ops row so that a retry gets the same answer. Guards are the rating_seq, mark_seq and turn_seq columns of migration 10.

Ledger split (by where the effect lands):

- Catalog ops table (migration 10), written in the same transaction as every catalog write: ratings, marks, turns, selection chunks. It is in the same database, so the two commit together.
- Server request_ledger in server.db, for effects outside the catalog: crop saves, Yandex saves, and POST commands that have no catalog row. Pending then done (section 3.5).
- No ledger: scan and import starts (a second start joins the running job), and versioned settings and session writes (idempotent by version).
- Retention: 24 hours or 10,000 rows per scope, whichever is smaller (P).

Client:

- One request in flight per item. While one is in flight, ratings keep only the last value and turn deltas sum mod 4 (a zero sum sends nothing). Each request carries the guard from the last acknowledgement. The window is 100 to 200 ms (P). Flush on acknowledgement, visibilitychange and pagehide.
- On 409 the client drops that item's queued intents for that field, shows a notice naming them, and re-reads the record from the 409 body. No silent retry.
- Pending record, written before the request leaves and deleted at a final answer: browser IndexedDB, desktop a JSON file written by write-then-rename. Fields: request_id, file_id, field, value, version, sent_at. On start: reconcile with GET /api/ops/{request_id} (applied, conflict or unknown), then by reading the item. Never resend automatically, except a set (idempotent). A turn is resent only with its ledger answer or from migration 10. If IndexedDB is cleared, the user is told reconciliation is impossible.
- Desktop exit waits a bounded time for unacknowledged writes, then shows any still unknown. No whole Preferences value is ever flushed.

Resolved in 1.6:

- Versioned sets with a guard, not last writer wins, for ratings and marks (after migration 10). Lens A's unguarded turn and lens B's last-writer rating are rejected.
- Turn form: section 3.6 and O14.
- The reserved routes DELETE /api/items/{file_id} and POST /api/items/{file_id}/move are retired; there are no quarantine routes (quarantine is dropped, `WEB_CLIENT_TECH_DECISIONS.md:200`).
- Request keys are named request_id everywhere. The name `op` is retired (writes); transport's op is renamed.

### 1.7 Selections and jobs

Select-all (option A): `POST /api/selections/apply {listing, seq, count, set_digest, change, request_id}`. One read transaction recomputes count and set_digest over the current membership and collects the FileIds in final order. Mismatch: 409 set-changed with the new seq and count. Match: the collected ids go into a job in chunks of 500, each chunk one IMMEDIATE transaction on the mutation connection, FULL, with its ops rows and the job cursor. The job's as_of is the read's seq.

Explicit (option A): `{listing, ids (at most 4,096), change, request_id}`. Each id is probed against the membership condition in one read transaction. Any non-member gives 409 selection-changed, and nothing is applied.

Option B keeps transport's range form: `{listing, base, ranges, add, remove, change, as_of, expected_count, request_id}`, with ranges, add and remove counting together to 4,096 (a range counts as one).

Common rules:

- Body cap 128 KiB, the selection body cap in `WEB_CLIENT.md` (Limits). (A maximal selection of six-digit ranges is about 65.5 KB; 64 KiB would refuse a legal selection.)
- Members are materialised into job-owned catalog rows at creation (8 bytes each; about 1.5 MB at 189,518 members, arithmetic), so the job does not depend on the snapshot.
- Per member, the job applies only if its field stamp is at most as_of; otherwise the member is skipped and counted as a conflict, and a sample of 100 is kept.
- `change` is rating(value), mark(marked), turn(delta) or save_crops(folder token). save_crops has no per-member guard; its count is the guard. Its ledger rows go to server.db.
- Status moves on the `jobs` scope and in GET /api/jobs/{job}. `POST /api/jobs/{job}/cancel` stops at the next chunk boundary; committed chunks stay. One running job per device and two per server (P).

Resolved in 1.7: the 4,096 cap is the selection cap (ASSEMBLER; writes' 5,000 is rejected for one cap, as validators asked; the quarantine plan that shared it is dropped, `WEB_CLIENT_TECH_DECISIONS.md:200`). Lens A's 64 KiB and ops' 16 KiB are rejected (they refuse legal selections). Lan's 1 MiB is rejected (no use at 4,096 entries). Check-then-materialise races are rejected in favour of collecting in one read transaction.

### 1.8 Records, items, settings, views, inbox, tree, library, Yandex

- Records (binary, GridRecord): file_id u64; mtime u64; shown w and h u32; rating, borders, ext, kind, flags, orientation, rotation, pending, placeholder_turns u8; thumbhash length u8 and up to 25 bytes (THUMBHASH_MAX, analysis.rs:138); rating_seq, mark_seq, turn_seq, changed u64. duration_ms is removed (video not designed). 91 bytes at most without cut sides, 107 with (arithmetic). `placeholder_turns` = ((files.rotation - analysis rotation) + pending) mod 4 (state.rs:85-96). Option A: `POST /api/records {ids}`, at most 500 decimal ids. Option B: range reads, at most 1,024.
- Item: `GET /api/items/{file_id}` answers the viewer record in JSON, read in one transaction with the grid fields. Versions and mtime are decimal strings.
- Settings: `GET /api/settings` answers a whitelist of keys, never the auth store. `PUT /api/settings/{key} {value, version}` sets one typed key (versioned; the store is in section 3.4). Write_back is server-wide and owner-only. A version mismatch is 409 version-mismatch with the current value. A failed write changes neither the database nor memory and answers 503 not-persisted (507 on SQLITE_FULL). The client keeps its value pending and posts a notice.
- Views: `GET /api/views`; `GET`, `PUT`, `DELETE /api/views/{name}`. A view's ETag is the BLAKE3 hex of the canonical JSON of {folder, recursive, query, sort} (sorted keys, UTF-8, no whitespace), quoted in the header. PUT requires If-Match and answers 412 precondition-failed on mismatch; create uses If-None-Match: *. Two devices saving one name overwrite each other today (views.rs:51; browse/views.rs:26-33).
- Inbox: `GET /api/inbox`; `POST /api/inbox/entries/read` and `/dismiss` (lists; report how many were not applied); `POST /api/inbox/retries/{pass}`; `GET /api/inbox/ledgers/{kind}?limit=&seen_to=`; `PUT /api/inbox/viewing {viewing}`. The flag is per device record and the inbox is viewed while any device reports it. A flag not refreshed within 60 s (P) lapses, and it clears on revocation. The flag is process-wide today (inbox.rs:431); per device fixes that.
- Tree: `GET /api/tree`, a flat pre-order DTO with an ETag. A folder in a listing is named by its Merged parts (rules.rs:49-57); a library folder by its RootId. Names are JSON strings, or `{b: hex}` for non-UTF-8 (name.rs:230-238).
- Folders a phone names: opaque 128-bit tokens issued for an entry of the server's own folder listing (`GET /api/fs/roots`, `GET /api/fs/folders?at=<token>`, folders only under the scanner's refusal rules), bound to the device and expiring after 10 minutes (P). The desktop's native picker obtains a token by asking the server for the picked entry. Raw paths are never sent, since library.rs:188 and yandex.rs:144 write into the folder they are given.
- Library: `GET /api/library/status`; `PUT /api/library/paused`; `GET` and `POST /api/library/roots` (POST takes a folder token); `DELETE /api/library/roots/{root}`; `POST /api/library/scans {roots}`; `POST /api/library/scans/stop {root}`; `POST /api/library/rescan`; `GET /api/library/write-errors`. Status moves on `jobs`.
- Yandex: `POST /api/yandex/searches {file_id}` answers `{sid}` (the server resolves the path); `GET /api/yandex/searches/{sid}` answers the status and the full result list so far, non-consuming (yandex.rs:485-489 consumes today); `DELETE /api/yandex/searches/{sid}` cancels; `GET /api/yandex/searches/{sid}/results/{id}/image` is private and immutable for that sid and id; `POST /api/yandex/saves {sid, results: [decimal ids], folder: token}`, where the server takes each URL from its own map (yandex.rs:336-341 fetched a supplied URL, an SSRF); `GET /api/yandex/save-folder`. A deleted search answers 404. The `yandex` scope bumps on any change. A result folder is owner-wide (O31).

### 1.9 Schema and migrations

Numbering follows TO_9 (schema.rs:36-44, 350). Migration 10 is the union of the cluster positions; its timing is O17.

- Files rebuilt as `id INTEGER PRIMARY KEY AUTOINCREMENT`, ids kept. The procedure is in section 4.7. A reused id would show another photo under a stale snapshot and under an old thumbnail key. Pair merges already delete rows without AUTOINCREMENT (pair.rs:345, 434).
- `rating_seq`, `mark_seq`, `turn_seq` on files (INTEGER NOT NULL DEFAULT 0, seeded from the catalog's sequence). rating_seq: triggers on insert, on update of rating (with WHEN), and on delete. mark_seq: triggers on border_marks insert and delete; set_border_mark uses ON CONFLICT(file_id) DO NOTHING (borders.rs:22-31), so a same-value mark does not fire. turn_seq is set in queue_rotation (queue.rs:103-127) in the same transaction.
- Stamp extensions to files.changed (never files.seq): membership and shape columns, file_writes, border_marks and analysis.error (section 4.6).
- `file_tombstones`, meta rows `bulk_seq`, `tombstone_floor`, `move_seq`, and `taken_zone`.
- Catalog ops table (section 1.6); job state and cursor tables in the catalog, so each chunk's cursor and writes commit together. The outcome report of a job is `job_reports` in server.db (writes).
- The replacement rule: the rating row is kept (O24); the mark is removed by a plain DELETE, which fires its trigger.

### 1.10 Route table and body caps

| Routes | Gate | Body cap |
|---|---|---|
| GET /login | pre-login | 0 |
| POST /api/auth/login | pre-login | 4 KiB |
| POST /api/auth/logout, GET /api/auth/session, POST /api/auth/elevate | LAN session | 0; 0; 4 KiB |
| POST /api/auth/password/setup (loopback), change (LAN or loopback), reset (loopback), sign-out-everywhere | per section 2.8 | 4 KiB |
| GET /api/devices, PATCH/DELETE /api/devices/{id}, POST /api/devices/{id}/revoke, POST /api/devices/revoke-others | per section 2.10 | 0; 4 KiB |
| GET /api/hello | loopback, no bearer | 0 |
| POST /api/server/stop | loopback bearer, build-exempt | 0 |
| GET /api/settings; PUT /api/settings/{key} | session or bearer | 0; 4 KiB |
| PUT /api/session/listing | session or bearer | 4 KiB |
| POST /api/changes | session or bearer | 16 KiB |
| GET /api/tree; GET /api/items/{file_id} | session or bearer | 0 |
| PUT /api/items/{file_id}/rating, PUT .../mark, POST .../turns | session or bearer | 16 KiB |
| POST /api/listings/full, /delta, /facets (option A) | session or bearer | 16 KiB |
| POST /api/records (option A) | session or bearer | 16 KiB |
| POST /api/selections/apply | session or bearer | 128 KiB |
| POST /api/thumbs/wants | session or bearer | 64 KiB |
| GET thumbnails; GET images | image (cookie or bearer; no build header) | 0 |
| GET/PUT/DELETE /api/views/{name}; GET /api/views | session or bearer | 0; 16 KiB |
| GET /api/inbox, entry actions, retries, ledgers, PUT /api/inbox/viewing | session or bearer | 0; 16 KiB |
| GET /api/jobs/{job}; POST /api/jobs/{job}/cancel; GET /api/ops/{request_id} | session or bearer | 0 |
| Library routes (status, roots, scans, rescan, paused, write-errors) | session or bearer | 4 KiB; 0 |
| GET /api/fs/roots, GET /api/fs/folders | session or bearer | 0 |
| Yandex searches and saves; GET save-folder | session or bearer | 16 KiB; 64 KiB; 0 |
| GET /notices | session | 0 |

Every route declares its cap. The table is the input to the route-table test. Reads of bodies use the cap plus one byte (fetch.rs:49-52 pattern).

### 1.11 Resolved conflicts (cluster 1)

- Change channel: transport's vector and listing's entries are one POST (1.1). Lan's GET with a body-less vector is rejected.
- Pre-login set and handshake: section 1.2. The LAN never serves the hello.
- Attach: envelope 1 and build_id, plus the proof, after the lock probe (section 3.2). Build equality alone (events D4) is rejected because the decided item requires the protocol integer; lan's build-only rule is rejected for the same reason.
- CSRF: X-Pg-Client on unsafe methods plus Origin (1.2). Lan's X-Gallery-Build as one header for both jobs, identity's X-Gallery-Request and the derived token are rejected. Lan's X-Pg-Bundle-only fetch path for images is rejected, since image GETs need no header.
- Routes: `/api/auth/*` for sign-in; /api/hello and /api/server/stop; /api/changes; /api/items; /api/listings/full|delta|facets and /api/records under option A. The /api/login, /api/me, /loopback/* and /api/handshake names are retired.
- Error codes: section 1.4. version-mismatch is not frozen; the body is `{error}` everywhere.
- Thumbnails: section 1.5.
- Cache-Control: O11.
- Ops ledger: split by effect (1.6). Writes' request_ledger in server.db is kept for non-catalog effects; the transport catalog ops table is kept for catalog effects; lan's ledger-in-catalog is adopted for those.
- Migration 10: contents are the union (1.9); timing is O17.
- Caps: 4,096 entries and 128 KiB, shared (1.7).
- Desktop heights quantised (O13).
- Full-image views: section 4.12.

### 1.12 Could not verify (cluster 1)

- Chrome on Android (untested): whether an img reuses a displayed 1x1 placeholder for the same URL (a nonce may be needed); whether an img is cancelled when its element is removed; whether Clear-Site-Data with the page open takes effect; a 307 to an immutable target, including a chain if the target was evicted between the two (bounded by the browser's redirect limit).
- Whether a ready signal can precede the placeholder response on Chrome; the race rule removes the hazard, and its frequency is unmeasured.
- Qt wasm held fetch: the 25 s hold with the screen off, and the order of visibilitychange, pageshow and online on resume (read in source, qnetworkreplywasmimpl.cpp:259, 276; not run).
- ureq holding a 25 s poll on a worker thread (the `ureq` package in Cargo.lock); the desktop's image path to loopback thumbnails.
- axum, hyper, h2 and tokio are not in Cargo.lock: the rapid-reset fixed versions, hyper's stream default and the blocking-pool default are unchecked. The stream cap and reset rule depend on them.
- The Qt wasm WebSocket has no ping (validator; not read here, and not relied on after O27).
- SQLite atomicity across an attached database under WAL: the bundled sqlite3.c (3.54.1, the fork rev ac441bb; `Cargo.lock` libsqlite3-sys 0.40.0, rusqlite 0.43.0) shows no super-journal count for WAL, so ATTACH is treated as non-atomic. No catalog write depends on it.
- The AFTER UPDATE trigger with WHEN on an upsert: untested. The mark path uses ON CONFLICT DO NOTHING.
- Stamp completeness and scanner rebinds (pair.rs:355-363); same-size, same-mtime replacement is the catalog's own limit.
- Heights S, their ratio and mean sizes at each height (only 29.6 KB at 384 px is known, THUMBNAILS.md:34).
- Background lane CPU priority; the cache-disabled answer (ThumbCache::disabled, lib.rs:115).
- Measurements M0 to M10, the 25 s hold, the 250 ms batch, the 10-minute windows and every cap here are proposals or arithmetic. Nothing was built or run.

---

## 2. Identity: sign-in, devices and the server store

### 2.1 Store and locks

- One SQLite file, `<home>/server.db`, in WAL mode. Synchronous FULL on the mutation connection; NORMAL for login_failures and batch connections (section 5.13). Opened through Layout::for_sqlite (layout.rs:87).
- Placement. ASSEMBLER: home, not catalog_dir. The lock and run files sit in catalog_dir, because the catalog folder is the one-writer unit. Credentials and key material stay out of the catalog folder, which migration backups and rebuilds copy (lan), and the one store lives beside gallery.json, the preferences home (layout.rs:37-38). Identity's catalog_dir placement and writes' separate auth.sqlite are rejected; pg-auth owns the schema, not a separate file.
- Two catalog folders under one home. ASSEMBLER: refused by `<home>/home.lock`, taken exclusive for the server's life. This answers the validators' open question; without it, two servers would share one device table and one LAN port.
- Locks: `<catalog_dir>/server.lock` is taken non-waiting. The server holds it exclusive for life. CLI writing commands and the pre-cutover desktop hold it shared (section 3.2).
- Judge (pg-dbfile judge.rs:107) reads server.db under Live::shared. Live::exclusive is taken only to replace the file (section 2.2). The identity design's exclusive probe on every start is removed (validator).

### 2.2 Verdicts at start

| Verdict | Action | LAN |
|---|---|---|
| New (judge.rs:113-114) | Create the store. Seed the remembered rows from gallery.json when it parses, else defaults. If the catalog meta row server_store is set, post a store-missing notice. | off until an owner row exists |
| Usable | Open it. | per settings and owner row |
| Damaged (judge.rs:120) | Under Live::exclusive, move_aside to server.db.damaged (remove.rs:21, never overwrites), create as New, post a notice. | off |
| Foreign (judge.rs:131-132) or Differs (judge.rs:134-135, including a newer schema) | Do not touch the file. Exit with the reason; the desktop shows it. The owner moves it aside with the offline command `gallery store move-aside`, or runs the newer build. | not started |

The catalog meta row server_store is set once, when the server first creates server.db. A missing server.db with that row set means the store was lost; the notice says that sign-ins, devices and the password are gone.

### 2.3 Tables (server.db)

- `meta(key, value)`: schema marker and the cutover import marker.
- `owner(id CHECK id = 1, phc, epoch, changed_ms)`: one row. Its existence is the owner-set marker. `epoch` changes on every password change.
- `devices(id CHECK length 32, kind CHECK in (this_computer, browser), name, created_ms, last_seen_ms NULL, revoked_at_ms NULL)`. `id` is the BLAKE3 digest of the 32-byte device key (the key itself is the device cookie's value and is never stored).
- `sign_ins(digest BLOB PRIMARY KEY, device_id UNIQUE REFERENCES devices ON DELETE CASCADE, owner_epoch, created_ms, last_used_ms, elevated_until_ms NULL)`. `digest` is the BLAKE3 of the 32-byte secret; one sign-in per device.
- `login_failures(source PRIMARY KEY, failures, last_failure_ms, next_allowed_ms)`.
- `remembered(scope_kind, scope_id, field, version INTEGER NOT NULL, value JSON, updated, PRIMARY KEY (scope_kind, scope_id, field))` (section 3.4). Preferences, LAN settings and write_back live here.

Catalog data stays in the catalog. Views are CARRIED by rebuild (rebuild.rs:148-149). Credential data is not in the catalog.

### 2.4 Cookies and CSRF (ASSEMBLER reconciliation)

- `__Host-gallery_session`: 32 random bytes, base64url; stored only as the BLAKE3 digest. Path=/, Secure, HttpOnly, SameSite=Lax, no Domain. Max-Age equals the absolute lifetime (O2: 7776000 s at 90 days). The server enforces idle and absolute lifetimes from the row.
- `gallery_device`: the device key itself, which is a secret (the server stores only its BLAKE3 digest in `devices.id`); it grants nothing on its own. Path=/api/auth/login, Secure, HttpOnly, SameSite=Lax, Max-Age 34560000 s (400 days, refreshed at login). It has no `__Host-` prefix because its Path is not /.
- Why two cookies. Logout clears the session cookie. Without gallery_device, the next login cannot find the device, its preferences and its budget, and it would create a new device (the gap the validators found in the single-cookie design). Rejected: one cookie (transport and lan) for this reason; device's `__Host-pg_sign` and `pg_device`; identity's SameSite=Strict.
- SameSite=Lax on both. Strict is rejected: it withholds the cookie on top-level cross-site navigation, which forces a login on every external link (writes), and the decode amplifier is closed by the Sec-Fetch-Site refusal (1.2 step 4), not by SameSite. Lax is also what cluster 4 and cluster 5 assume.
- CSRF: no derived token (X-Gallery-Csrf is rejected). The unsafe-method rules are 1.2 steps 3 and 4: Origin, application/json and X-Pg-Client. A cross-origin page cannot send X-Pg-Client without a preflight the server never answers.

### 2.5 Login

- `POST /api/auth/login {password}`, 4 KiB cap, after the gate steps 1 to 5, 8 (X-Pg-Bundle and X-Pg-Client). The per-source throttle is checked before any hashing (section 2.11).
- Verification runs through the queue, reading owner.phc and owner.epoch; the epoch is kept as e. A wrong password gets a generic 401 with the same body shape as every auth failure.
- A correct password runs one BEGIN IMMEDIATE transaction: (1) read owner.epoch; if it differs from e, return the generic 401. (2) If gallery_device names a live browser device, reuse it; otherwise count live browser devices, refuse at 32 with 409 device-limit, and insert a new browser device. (3) Delete the device's existing sign-in and insert the new one with owner_epoch = e and created_ms = now. (4) Clear the peer's login_failures row.
- Commit, then respond 200 with both Set-Cookie headers, `Cache-Control: no-store`, and `{authenticated: true, device: {id, name, kind}, elevated_until_ms: 0}`. A commit failure answers 503 with no cookie.
- Rehash on success when the stored Argon2id parameters are weaker: UPDATE owner SET phc WHERE epoch = e; no epoch bump.
- There is no in-memory sign-in registry. Writers serialise in SQLite, so the epoch check inside the transaction orders a login against a password change.

### 2.6 Per-request session check

1. In memory: a device id in the deny set, or signins_fenced set, answers 401.
2. One indexed query on the read pool: sign_ins by digest, joined to devices and owner. Must have revoked_at_ms NULL, owner_epoch equal to owner.epoch, idle under 30 days and absolute under the lifetime; otherwise 401 not-authenticated.
3. elevated_until_ms is read from the same row.
4. last_used_ms is written when more than an hour old; devices.last_seen_ms on the first authenticated request and then hourly. Writes are best effort. now_ms is the server's wall clock, passed in. A clock set backwards extends sessions.

Elevation (`POST /api/auth/elevate {password}`, verified through the queue and limiter) sets elevated_until_ms to now + 10 minutes. No rotation; sleep ends it because the clock is wall-clock.

### 2.7 Logout

`POST /api/auth/logout` deletes the caller's sign-in row and nothing else. The response is 204; it clears `__Host-gallery_session` (Max-Age=0) and sends `Clear-Site-Data: "cache", "storage"` only, never "cookies", which would also clear gallery_device. The page also deletes its own Cache Storage and IndexedDB entries, because Android's handling of Clear-Site-Data is unverified. The device and its preferences survive logout.

### 2.8 Password operations and sign-out

- `POST /api/auth/password/setup {new}`: loopback only, only when no owner row exists (else 409).
- `POST /api/auth/password/change {current, new}`: from a LAN sign-in or loopback; the current password is required.
- `POST /api/auth/password/reset {new}`: loopback only, no current password; also clears login_failures.
- `POST /api/auth/sign-out-everywhere`: elevated (O1); keeps devices.
- Change, reset and sign-out-everywhere run the fence procedure (2.9), then one transaction: owner.epoch + 1 (with the new phc for change and reset), DELETE every sign_ins row. Devices are kept. The caller's response clears the session cookie. Each posts a notice.
- Rationale: keeping devices means a stolen phone costs one password change, not every phone's preferences.
- Resolved: deleting every other device (auth) and sign-in-only delete that keeps the caller (device) are rejected. The CLI gallery auth set-password is setup when no owner row exists and reset otherwise.

### 2.9 Fence, drain and commit (revoke, change, reset, sign-out-everywhere)

Lock order: the registry lock (the fence's mutex; taken briefly and never held while waiting), then the per-device write gates (async read-write locks, in device-id order), then the write transaction. A mutation holds its device gate shared across its check and commit, and reads the deny set while holding it.

1. Fence: insert the device id into the deny set (revoke), or set signins_fenced (change, reset, sign-out-everywhere). Release the mutex.
2. Drain: take the affected browser device gates exclusively, in device-id order.
3. Commit one BEGIN IMMEDIATE transaction. Revoke one device: `UPDATE devices SET revoked_at_ms = now WHERE id = ?` and DELETE its sign_ins row. The revoked device row is kept (lan's delete-the-row is rejected: the revoked row stops a revoked phone's old identity from being reused). Change, reset or sign-out-everywhere: UPDATE owner ... WHERE epoch = the verified epoch; zero rows changed answers 409 and the caller retries; then DELETE sign_ins.
4. On success: clear the fence entries; answer that device's held polls with 401; cancel its open image streams; release the gates; post the notice; answer.
5. On failure: keep the fence, release the gates, answer 503 with the reason that the change holds until the server stops, post a notice, and retry the commit with backoff until it succeeds. The LAN stays up for other devices. The device list shows the device as revoking while fenced.

Consequences: a login that verified the old password fails its epoch check inside its own transaction once a change commits. A restart clears the fence and the database is authoritative; a restart during an uncommitted revoke reinstates that device (recorded residual). A restored older server.db revives revoked devices and an older password; not detected in v1.

Resolved: commit-before-fence (device) and the flag checked later or relied on expiry (auth) are rejected. Closing the whole LAN until a commit succeeds (auth) is rejected.

Queued thumbnail work is not cancelled by revocation: it fills the shared cache, so revocation stops delivery to the device, drops its pending delivery entries and cancels its open image streams.

### 2.10 Devices

- Kinds. `this_computer`: the desktop; a random 128-bit id created at first server start and kept across restarts; never revocable (403 not-revocable); no sign-in; loopback principal Local. `browser`: one browser profile on one phone (tabs share it), created inside the login transaction. A second browser on one phone is a second device.
- A device owns its preferences and its per-device budgets. The sign-in is the authority and expires; the device persists until forgotten. The fixed key 'desktop' is removed everywhere.
- Names: at most 64 characters after trimming, with control, format, bidi-control and newline characters removed; empty takes the default. A browser default is an allowlisted label from the user agent (for example 'Chrome on Android'), computed at creation. The raw user agent and IP are never stored. The desktop default is the machine name (unverified: no hostname call exists in crates/).
- States: live; signed_out (no live sign-in); revoked (revoked_at_ms set); revoking (derived while fenced).
- Pruning (narrow). A browser device with last_seen_ms NULL, no sign-in, and created more than 24 hours ago is deleted at start and hourly: its login response was lost before any request, so it holds nothing. Devices with a last_seen are never pruned automatically. ASSEMBLER: lan's blanket never-prune line cites a confirmed item not found in WEB_CLIENT.md, and the general 24-hour prune (auth) would delete preferences; the narrow rule is the one both sides can accept.
- Forgetting: `DELETE /api/devices/{id}` forgets a revoked or signed_out browser device with its preferences. A live device answers 409; this_computer answers 403 not-revocable.
- Cap: 32 non-revoked browser devices (P), counted inside the login transaction; over it, 409 device-limit. Reuse of an existing device is never refused.
- Routes: `GET /api/devices` (state and fields); `PATCH /api/devices/{id}` (rename or prefs of the caller's own device); `POST /api/devices/{id}/revoke` (single revoke; O1); `POST /api/devices/revoke-others` (bulk; O1); `DELETE /api/devices/{id}`. The routes accept only 32-hex ids.

### 2.11 Rate limits

- Per source address, persisted in login_failures so a restart keeps the budget. After 5 consecutive failures the address waits 30 s, doubling per further failure to a 15-minute cap (P). A success clears the row; a row with no failure for 24 hours expires. Login, elevate and change share one budget.
- Globally, verifications are serialised; a failed verification delays the next by at least 1 s. A correct password is delayed, never refused.
- Verification (Argon2id, m=128 MiB, t=3, p=1, the default made in this pass; O3) runs on one dedicated blocking thread with a queue of 4. A full queue answers 429 with Retry-After before any hashing. A throttled address gets 429.
- If the failure row cannot be written, an in-memory counter applies for the process; the response is the same generic 401.
- Password: NFC-normalised; at least 15 characters after normalisation (O3); at most 1024 bytes; no composition rules; never logged or echoed.

Resolved: in-memory counters (device), ops' 15-minute window reset on restart, account lockout, CAPTCHA and trusting X-Forwarded-For are rejected.

### 2.12 Principals and the two routers

- Local: the loopback router. Credential: the bearer token. Its device is this_computer. Every route is reachable; elevation is always satisfied.
- Lan: the LAN router. Credential: `__Host-gallery_session`. Carries the device id, the sign-in and elevated_until_ms.
- Each listener builds its own router. The LAN router never accepts the token; the loopback router never reads a cookie. Route absence is the rule, not per-handler checks.

### 2.13 Elevated set (O1 and the rest)

LAN elevation applies to: enabling write_back (O1); revoking another device (O1); revoke-others and sign-out-everywhere (O1, bulk); a change of password (always needs the current password). Ordinary LAN sign-in covers reads, ratings, marks, rotation, scans, imports, Yandex, library folder add and remove, deletes and moves into quarantine, the device list, rename of own device, forgetting a revoked or signed-out device, logout, and disabling write_back. Loopback only: LAN enable and bind, port and certificate paths, password set or reset without the current password, server stop, offline move-aside, and quarantine purge (O7). Elevation is a timestamp on the sign-in row, so it needs no rotation.

Resolved: elevation for every delete (auth) and purge over the LAN (auth) are rejected; a LAN enable from a phone is rejected.

### 2.14 Preferences (one store, owner and device scopes)

The remembered table in server.db (section 3.4) holds everything below; the mirror is gallery.json.

- Server scope: write_back (default true, prefs.rs:101), lan.enabled (default false), lan adapter GUID, lan port, lan stable name, certificate paths (paths only; section 5.9), yandex_folder (O31), the cutover marker.
- User scope: show_borders, the remembered listing, last_file (O8).
- Device scope (this_computer): row_height, dev_mode, real_size (O29). Browser devices carry row_height and dev_mode only, never real_size.
- The write_back lock: each rating write reads write_back under a settings read lock held from before its transaction to its commit; a write_back change takes the write lock while it writes and sets memory. That gives one total order without a catalog row.
- Resolved: write_back in the catalog meta table (lens B) and in gallery.json (transport) are rejected; LAN settings in gallery.json (auth) are rejected, since a damaged store must read as LAN off.

### 2.15 CLI credential commands and the offline path

`gallery auth set-password` reads the password with echo off from a console only; it refuses otherwise and never reads argv or the environment. It enforces the 15-character minimum.

1. If Lock::try_take(server.lock) finds a server running (exclusive), call loopback setup, change or reset with the token from launch.json (one re-read of launch.json on 401). If loopback is unreachable, refuse with a message.
2. Otherwise take server.lock shared and write in one BEGIN IMMEDIATE transaction under SQLite's own locks. No exclusive hold on server.db is taken.

`gallery store move-aside` takes home.lock, then Live::exclusive on server.db, then move_aside (remove.rs:21). The CLI refuses the offline credential write under WSL against a Windows-mounted folder, because locks do not hold across operating systems (O6).

### 2.16 Notices, logs and redaction

- Notice variants are new members of the Notice enum (notice.rs, around line 352), posted to the desktop inbox (inbox.rs:341). They cover: LAN not started and why; store missing; store moved aside; store refused; password set, changed or reset; device signed in (new device only); device revoked; sign-out-everywhere; revocation not committed (device fenced, retrying); throttling engaged; device limit reached; preference mirror not written.
- gallery.log records device ids and names, route template, outcome, and the source address for failures. One redaction function strips Cookie, Set-Cookie and Authorization headers and never logs the bodies of /api/auth requests.
- Never logged: the session secret, the device cookie value, the token, H, the password, the PHC string, the verifier or any digest of them. The device id may be logged; it grants nothing.

### 2.17 Resolved conflicts (cluster 2)

- Store location: home, not catalog_dir or auth.sqlite (2.1).
- Lock: server.lock in catalog_dir, not Lock::take on the home folder (auth), `<home>`/server.lock (device), `<catalog>`.server.lock (ops), or a PID file. Non-waiting try (section 2.1).
- Exclusive hold on server.db: only for replacement. The auth probe on auth.sqlite and the device design's probe are withdrawn.
- Launch file: launch.json in home (section 1.3; B9). Its proof is kept.
- Desktop key: the random this_computer id. A fixed 'desktop' key and a token-derived key are rejected.
- Cookies: section 2.4. Strict is rejected; Max-Age is the absolute lifetime (lan's idle-based Max-Age is rejected: the browser would hold a cookie the server has idled out).
- Login reuses non-revoked devices; delete-and-recreate (auth) and creating a device before the password verifies (device) are rejected.
- Logout keeps the device; Clear-Site-Data uses cache and storage only.
- Password change ends sign-ins, keeps devices; sign-in-only delete with the caller kept (device) is rejected.
- Pruning: narrow rule (2.10).
- Rate limits persisted in server.db (2.11).
- Session lifetime numbers are owner questions (O2), not decided.
- The device's real_size row is this_computer only.
- Notices: section 2.16.

### 2.18 Could not verify (cluster 2)

- The argon2 0.5.3 closure (not in Cargo.lock; unicode-normalization not locked); getrandom and constant_time_eq are locked but not direct dependencies; blake3 keyed_hash and derive_key signatures; no build was run.
- Windows owner-only DACL and DPAPI calls: Win32_Security_Authorization is not enabled (the `windows-sys` entry in `[workspace.dependencies]`); the SDDL calls are unwritten; PG_HOME on another disk is untested.
- LockFileEx between two handles of one process (cutover design:890).
- The desktop machine name for the this_computer row: no hostname call exists in crates/; COMPUTERNAME and GetComputerNameExW are unverified.
- Android Chrome: acceptance of `__Host-` Secure SameSite=Lax cookies under the private CA; Clear-Site-Data storage effects; how many connections the login page opens.
- Argon2id time and peak memory at 128 MiB; the OWASP figures were recalled, not checked.
- The 400-day device cookie lifetime (general knowledge, not checked).
- WAL with synchronous FULL is assumed safe on power loss; WAL on removable or network folders is assumed to fail.
- Two Windows accounts with one catalog folder: the second fails the instance lock. Not tested.
- Measure before fixing: verification time and memory; queue wait behind a 20-attempt flood; per-request session lookup latency; FULL commit cost for login and revoke; time from revoke response to the first 401; last-used write rate.

---

## 3. Writes, cutover and view state

### 3.1 Cutover phases

- C1: non-waiting Lock::try_take and Lock::try_take_shared on server.lock, beside Lock::take (lock.rs:12-13, 27-38, which waits about 5 s). The exclusive try at core.rs:92 moves into the change branch (core.rs:91-92), so a prepare that needs no change never takes the exclusive hold; the refusal at core.rs:93-95 stays. The server takes the prepare lock (catalog.sqlite.lock, lifecycle.rs:43-46) for its whole life through an internal prepare body. No schema change. A second in-process desktop is refused (O22).
- C2: pg-web with routes for every inventory operation, run from tests; GET /api/status and GET /api/dups; the JsonStore durability fixes and the set_aside rename (store.rs:61).
- C3: GET /api/hello; launch.json; loopback order (1.2, 1.3); POST /api/server/stop; discovery and attach (3.2); the token DACL test (unverified).
- C4: the Loopback backend and the conformance suite run on both backends.
- C5: the change pump moves behind the trait. Direct implements the vector from the in-process Change bus; Loopback reads POST /api/changes on a worker thread. The resume, gap resync and retention text is deleted.
- C6: the remembered table in server.db; views, inbox and the thumbnail placeholder behind the trait; the gallery.json mirror written by the server. Direct still writes gallery.json. O29 is decided before this step.
- C7: CLI routing (3.8).
- C8, the flip: the default becomes Loopback in one step (reads and writes together). The first start imports gallery.json and the catalog session row into the remembered table, with the import marker in the same server.db transaction. The logon task ships (O5). The Direct lever is selected by environment variable (O23).
- C9: the Direct lever and the desktop's pg-app dependency are removed; tools/crate-layers.txt:48 and the boundary check change in the same commit.
- Quarantine: dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`, recorded in this pass). The Q1 to Q3 build phases no longer exist.

Resolved: "phase" is used for cutover steps (C1 to C9). Lens A's ambiguous phase 5 is removed.

### 3.2 Locks, prepare and attach

- server.lock: non-waiting, in catalog_dir. The server holds it exclusive for life. CLI writing commands hold it shared for their run; read commands that must run prepare do too; the pre-cutover desktop holds it shared for its life. desktop.lock (home) is exclusive for the pre-cutover desktop and refuses a second desktop process.
- Catalog holds: prepare takes the exclusive Live probe only in the change branch. The server keeps the shared hold for life (core.rs:124-125, kept by app.rs:62). The residual old-build risk is O19.
- Server start, in order: (1) try server.lock exclusive; if held, exit 75 (or 0 when a matching server answers hello). (2) Write server.holder (kind and pid) as an unlocked informational file. (3) Prepare under the prepare lock; refuse the rebuild-window state (catalog absent while catalog.sqlite.replaced exists, rebuild.rs:87-91). (4) Startup repair: delete scan leases (types.rs:89; roots.rs:221-223); leave writer temp files alone (they are the only whole copy of a write). (5) Import once if the marker is missing. (6) Write launch.json (temp-then-rename). (7) Bind loopback; the LAN listener only if the checklist passes (section 5.11). (8) Start the job, the writer and the inbox; after bind, the quarantine recovery job (3.11).
- Desktop attach (from C3): (1) try server.lock. If free, no server runs: run the task (`schtasks /Run`, section 5.3) and wait for hello with a matching build. The desktop never spawns the server and never kills a process (the cluster-3 detached spawn is rejected: lan's kill-on-close analysis, process.rs:104-115, and the two-supervisor problem). (2) If held, read launch.json, call GET /api/hello with a fresh nonce, verify the proof and envelope 1 and build_id, and only then send the bearer. A failed check means a squatter: the file is ignored and no bearer is sent. (3) On a build mismatch: once per desktop start, POST /api/server/stop with the bearer; wait for the lock within a bound; run the task. A second mismatch is an error state. (4) Held but no answer: show the connecting state naming the holder kind from server.holder, and retry within 5 s (P); never kill. A 401 later re-reads launch.json once.
- Before C8 the desktop default remains Direct and holds server.lock shared; it does not start the server.
- Exit codes: 0 (a matching server already holds the lock), 75 (the lock is held by a non-server writer or by a server with another build; the task retries), 1 (any other failure).

### 3.3 Migrations and rebuild

Migration timing is O17; contents are section 1.9. The AUTOINCREMENT rebuild and the files-stamp migration run inside prepare, under the exclusive hold only when a layout change is needed (section 4.7). Migration backup removes its .partial on failure (rebuild.rs:43-47) and is refused below the catalog size plus the free-space floor (section 5.13). A pre-migration build refuses a newer catalog (core.rs:75-80). The schema-neutral rollback holds up to migration 10 (O17, O18).

### 3.4 Stores after the flip

| Store | Owner from C8 | Contents |
|---|---|---|
| catalog.sqlite | server | files, folders, views, inbox rows, analysis, ratings, border marks, pairs, job tables, ops table |
| server.db | server | remembered (preferences, LAN settings, session listing, cutover marker), login and credential tables (section 2.3), request_ledger, job_reports |
| gallery.json | server writes a mirror; read only by a rollback build and by the first import | the seven keys: write_back, real_size, row_height, dev_mode, yandex_folder, show_borders, last_file |
| thumbs cache | server | thumbnails |
| library files | server | crop copies, XMP and turn copies, Yandex saves |

Remembered table: `remembered(scope_kind, scope_id, field, version INTEGER NOT NULL, value JSON, updated, PRIMARY KEY (scope_kind, scope_id, field))`, version 0 meaning never written. Each applied write is one FULL transaction that sets value and version + 1. After commit the server rewrites gallery.json through the JsonStore pattern (temp, sync, rename, temp removed on failure). A mirror failure after commit posts a notice and the write stays applied; the mirror is retried at the next write and at start.

Resolved: the cutover's whole-file JsonStore writes and the identity design's per-field writes are both replaced. The validator found JsonStore::update (store.rs:51-57) changes memory before the write, so the "file-first" claim does not hold; the design writes server.db first and the mirror after. Two stores with a write order (content in gallery.json, versions in server.db) and versions inside gallery.json are rejected.

The catalog session row (listing.rs:50) is copied from the table on clean stop so a rollback build reads a current value. The remembered row is the authority.

Settings that are one typed key per row (write_back, LAN) follow the same table.

### 3.5 Requests and responses

Three classes:

- Sets, by value and version: PUT rating, PUT mark, PUT settings, PUT session listing. A set whose value already equals the stored value answers unchanged and counts as applied, so a resend is safe. Otherwise 200 `{applied, version, record}`, or 409 version-mismatch with the current value.
- Commands, with request_id: POST turns (after migration 10 with a version), crop saves, Yandex saves, folder add and remove, job cancel.
- Views: PUT and DELETE with If-Match (section 1.8).

The request ledger (server.db table `request_ledger(request_id PRIMARY KEY, kind, state pending|done, params JSON, result JSON, created)`) runs in this order for each command:

1. A done row returns the stored result. The same id with a different kind or params is 422 refused, reason request-reused.
2. An absent row is inserted as pending (committed, FULL).
3. The effect runs in its own transaction (catalog) or file operation.
4. The row becomes done with the result (committed, FULL).
5. A pending row found on retry runs the reconcile rule for its kind: turn before migration 10: answered unknown, never re-applied; turn from migration 10: apply if files.turn_seq equals version, else 409 with the current record; quarantine commit: if a batch with the plan_id exists answer with it, else if the plan is in memory run the commit, else 410 plan-expired; crop copy and Yandex save: the output name is recorded in params before the write, and the output is done if it exists with the recorded size and mtime, else run again; folder add and remove and cancels and retries: idempotent by item state, run again; scan start: joins the running job.

Failures: SQLITE_FULL answers 507 storage-full with nothing committed. Other failed commits answer 503 not-persisted; the client keeps its value pending and posts a notice. Responses carry the record with its new seqs so the client adopts the versions.

Inbox notices are produced by the server and carry no request_id.

Resolved: the cutover's attached ledger in server.db and the quarantine request UNIQUE column are rejected; the catalog ops table (for catalog effects) and the server request_ledger (for the rest) replace them. The quarantine plan_id UNIQUE is an exact guard on the plan; it is not a second store.

### 3.6 Turns and ratings before and after migration 10

- Before migration 10: `POST /api/items/{file}/turns {delta, request_id}` is at most once; a pending ledger row on replay is answered unknown and never re-applied. Ratings and marks are unconditional last-writer-wins sets in the cutover release (unversioned; the decided versioned-set rule is not met until 10, and that is stated).
- From migration 10: the turn body is `{delta, version: turn_seq, request_id}`. The writer applies it only when files.turn_seq equals version, bumps turn_seq in the same transaction, and answers `{applied, record, version}`; a mismatch is 409 version-mismatch. The effective total is computed inside the writer from rotation and file_writes.value (queue.rs:103-120, grid.rs:26-29). Ratings and marks take version = rating_seq or mark_seq, applied only when they match. The owner question on this form is O14.
- Attribution caveat (unverified): when two clients send the same version, the one not applied can show the other's turn as its own. The no-double-turn guarantee holds.

### 3.7 Durability and atomicity

- User-mutation commits and request_ledger rows run on a dedicated pool connection with PRAGMA synchronous = FULL (ops decision 36). The setting is per connection, so the mutation connection is separate.
- Catalog read and scanner connections stay NORMAL (SETTINGS, schema.rs:377). Batch writes and thumbnail puts stay NORMAL. server.db is FULL on every connection except login_failures (NORMAL).
- No ATTACH. A WAL database contributes no super-journal count (aMJNeeded WAL = 0, sqlite3.c:88841-88850), and two WAL files commit per file (sqlite3.c:88889), so an ATTACH across catalog.sqlite and server.db is not atomic on crash for this build. Writes that span both use the ordered pending and done protocol (3.5).

### 3.8 CLI

- Writing commands (scan, import, remove, thumbs, borders, write, and the rest): try server.lock shared without waiting. On success, run the direct path for the whole command; nothing is refused. On failure a server holds the lock exclusively: read launch.json, call hello, verify the proof, and call the server's API with the bearer. Report job progress; Ctrl-C posts POST /api/jobs/{job}/cancel. If hello does not answer within 5 s (P), fail with 'server is starting, retry'. The command never checks for a server and then writes; the lock chooses the path.
- Read commands use Catalog::open (cli lib.rs:136 today, moved below the mode check). They never prepare while a server runs. When the catalog needs a change and no server runs, they take server.lock shared and run prepare.
- The CLI never stops a server.
- A long CLI write holds the shared lock, so a server start during it exits 75 and the task retries; the desktop shows 'server not running' and retries. O21 decides whether the start waits instead.
- A pre-flip CLI write beside an in-process desktop: O20.

Resolved: ops' exclusive-wait direct write (a refusal in disguise) and cutover's refusal of CLI writes while a desktop runs are rejected. Wire's gap (no CLI path) is closed by this rule.

### 3.9 View state (client)

Fields and stores:

| Field | Scope | Store |
|---|---|---|
| listing | remembered, user | remembered table; catalog session row copied on clean stop |
| show_borders, last_file | remembered, user | remembered table; mirrored |
| row_height, dev_mode | remembered, device | remembered table; mirrored |
| real_size (desktop only) | remembered, device this_computer | remembered table; mirrored (O29) |
| current, viewer_open, filters_open, scroll anchor, format | session | sessionStorage on the phone; memory on the desktop |

Not in ViewState: yandex_folder (a server setting); StartGate (start.rs:36-47); Deadlines (deadlines.rs:33-34); generation and reveal (controller.rs:204-208); grid_width (controller.rs:203); GridSize scale and shown_thumb (size.rs:40-44); saved_row; Input count; the listing id; modal panels (panels.rs:79-80) in v1; apply_view and StartOverrides (run-only); the filter bar's Counts.

- Start: StartSource returns Start {view, saved_row, session_known, apply_view} before Controller::new. Sources: argv (controller.rs:38), the saved view for --apply-view, the session listing (controller.rs:51; GET /api/session/listing from C8), and prefs (controller.rs:55; GET /api/settings). A failed read gives session_known = false, a visible notice and a retry; it never writes a default. PG_RECORD (controller.rs:57) stays desktop-only. No ViewState is captured before Started.
- Save rule: a listing write carries a version from a successful read (session_known) or from the client's own last successful write, and the server enforces it. A selection change while session_known is false is kept in memory, the read is retried, and the write follows with the read's version. If the retried read shows a newer listing, a conflict notice asks the user to keep theirs or take the server's. SaveLastFile is a separate command.
- Only the view is saved; StartOverrides never reach the saved session (O39). SaveSession writes the stored listing, not the start listing with overrides.
- Session-only copy: sessionStorage under gallery.session-view, whole-or-nothing JSON with format and build; every access wrapped in try/catch; a format or build mismatch discards it. localStorage is not used (it is shared across tabs). The server never stores session-only fields.
- Flush: debounced saves (1,000 ms, deadlines.rs:25-26) flush on visibilitychange (hidden) and pagehide through fetch with keepalive true, carrying X-Pg-Bundle. navigator.sendBeacon is not used, because it cannot set the header. Up to 1 s of last-file and session changes may be lost when Android kills the page (O10). There is no socket (events D14).
- Scroll anchor: the topmost visible FileId plus a fraction validated in [0, 1] (NaN and infinity rejected). QML reports it at rest, after a 300 ms pause (P), never per frame or as pixels.
- Restart: the desktop persists no ViewState and saves no default; it shows connecting until Start arrives.
- Overlay: session-only fields apply after Started and after the list is shown for its generation. current selects its FileId if present, otherwise the selection moves as WEB_CLIENT.md:218-219 says. The overlay never touches the start gate.
- Replay: Start derives Serialize and Deserialize (controller.rs:37-38) and round-trips through the replay Init record; the replay format bumps from 3 to 4 (replay.rs:13).
- Commands on the wire (21 variants, controller.rs:144-192): OpenList, Rate, Turn, ToggleBorderMark (sends the requested value; never computes !marked, shell.rs:486-487), SetBorderMark, RefreshTurned, SaveSession, SaveLastFile, SaveDevMode, SaveShowBorders, Size::SaveRowHeight, and the thumbnail height parameter. All others are local only.
- Gate before any bridge rewrite: generate the member manifest (272 members) and check the count; generate a stub per bridge type where every member throws and records its name (qmltest.py:48-60); a test runs Main.qml to Started and ListReady; record the touched set before Started; Rust data-layer stubs (call counting; failing reads that must emit no save; a stale ListShown before a Rate).

Resolved: the cutover's whole-value flush at exit is rejected (a stale copy reverts another device's change, store.rs:51-57; prefs.rs:147-156). Restoring the viewer is rejected for v1 (O38). A persisted browser Z is rejected.

### 3.10 Desktop writes

Desktop writes after the flip are sets and commands over loopback with the same guards. Exit waits a bounded time for unacknowledged writes, then drops unsent ones with a notice; a write still unknown at timeout is shown as unknown. No whole Preferences value is ever sent.

### 3.11 Quarantine (dropped)

Quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`, recorded in this pass). The store, the quarantine tables and triggers (migration 11), the three-step executor, recovery, purge, the quarantine routes, the UI and the test plan that stood here are removed. A user delete goes to the Windows Recycle Bin.

### 3.12 Resolved conflicts (cluster 3)

- Thumbnails: the image GET never queues; the miss returns the 1x1 placeholder; the want list queues (1.5). Writes' GET-side queue and ThumbHash placeholder are rejected.
- CSRF: X-Pg-Client on unsafe methods; X-Pg-Bundle and X-Pg-Build are build-gate headers only; X-Gallery-Request is retired (1.2).
- Cookie: section 2.4.
- Desktop key: the this_computer id, not 'desktop' (writes 3.1 changed).
- Yandex folder: owner-wide with a folder token (3.13 below). Writes' per-desktop path is rejected.
- Write_back and the session listing: the remembered table in server.db (section 3.4).
- Device and login records: server.db (identity; section 2.1), not auth.sqlite (writes); writes' statement that server.db holds only remembered, request_ledger and job_reports is withdrawn.
- Stop: POST /api/server/stop, exempt from the build gate, loopback only (writes' POST /api/stop is renamed).
- Cancel: POST /api/jobs/{job}/cancel (transport's DELETE is renamed).
- Prepare lock: waits about five seconds (LOCK_TRIES 100 at LOCK_WAIT 50 ms, lock.rs:12-13) and then refuses. "Fails at once" is corrected.
- Migration 10 contents: union (1.9). Timing: O17.
- Failure answers: 503 not-persisted and 507 storage-full, with memory unchanged (3.5).
- Ratings and marks: unversioned in the cutover release, versioned from migration 10. The decided item is marked amended, not conforming.
- Handshake: GET /api/hello on loopback, outside the LAN pre-login set; the LAN page checks bundle_id from its own page.
- Q-CLI: the decided no-refusal text governs while a server runs; the pre-flip case is O20.

### 3.13 Yandex on this surface

The desktop keeps the native folder picker. The picked entry becomes a folder token through the server (1.8). Saves take the token; the server normalises the path with pg_platform::names::normalize_path and saves through the same save code. Whether a phone may save is O30. Yandex saves write results into the owner's folder (O31).

### 3.14 Could not verify (cluster 3)

- Win32, none run: the token DACL (no DACL code exists in the repo); SetFileInformationByHandle with FileRenameInfo and ReplaceIfExists FALSE on a DELETE handle; FILE_DISPOSITION_INFO with the read-only attribute; sharing and error-code classes (32, 33, 5, 112, 39, 3, 21, 1167 are proposals); ERROR_NOT_SAME_DEVICE from MoveFileExW with flags 0; NTFS rename durability; MOVEFILE_WRITE_THROUGH for the mirror; LockFileEx between two handles of one process.
- Whether Rust std's try_lock on Windows locks the whole file (so a held server.lock's body cannot be read). The design avoids the question with the unlocked server.holder.
- The AUTOINCREMENT rebuild with foreign keys on (rebuild.rs:181-210 not read); the meta sequence name used to seed the seq columns.
- Folder add idempotency by normalised path (roots.rs not read).
- The turn attribution case (3.6).
- ureq holding the GET /api/changes long poll; the browser's fetch keepalive with a custom header on pagehide; whether Android Chrome delivers pagehide before a kill; sessionStorage survival.
- Whether the pre-cutover readers accept the gallery.json mirror as written (prefs.rs:147-156 not re-read).
- Request ledger retention and growth under scan-rate volume (unmeasured).
- Pairing: the unpaired stamp gap at pair.rs:355-362 is the listing cluster's; a held file that becomes listable through pairing can be missed by a delta until a digest check.
- The Start payload in a real run, the serde compile of ViewState and Fraction, the replay test, and the replay.rs:16-21 Init embed.
- The old-build residual (O19): no test or lock stops a pre-cutover process that takes a shared hold beside a running server.

---

## 4. Listings and the 1:1 control

### 4.1 Branch and what the design needs

Option A (O16, recommended by this cluster): a client-held replica. No server listing state; the server answers pure functions of the request body and the catalog. Option B (transport's working value and the decided stamp): server-held frozen membership, cap of 4 per device, release on disconnect, revocation and a 16-byte-per-item header (3.03 MB at 189,518 items; 12.1 MB for four), with wire routes and listing scopes. Sections 4.2 to 4.10 describe A. Where B differs it is said. B's header and scopes are in sections 1.8 and 1.1.

### 4.2 Identity

- listing_key: the first 32 lowercase hex characters (128 bits) of BLAKE3 over the serde_json bytes of the server-parsed Listing (crates/model/src/view.rs:11-19). BLAKE3 is a workspace dependency (`blake3` in `[workspace.dependencies]`). The client never supplies it.
- The key is build-scoped: a new build may serialise a Listing differently, so a replica held across an upgrade is orphaned and the next request is Full. A byte-pinning test on a fixed Listing guards this.
- Parsing is strict: an unknown field or unparseable date is 400 bad-listing. The lenient date defaults (query.rs:199-201, 211-214) are not used for keys. A key over 4 KB is 413.
- seq is meta.seq, read in the same read transaction as the rows it describes (sql.rs:17-23). catalog_id is meta.catalog_id (schema.rs:702).
- Stamp under A: (listing_key, seq). The controller generation (controller.rs:204-205, 334-337) stays a client request id and is never sent.

### 4.3 Routes and reply format (option A)

| Route | Body | Reply |
|---|---|---|
| POST /api/listings/full | {listing, anchor?} (anchor is a decimal FileId) | Full reply |
| POST /api/listings/delta | {listing, seq, catalog_id} | Delta, or Full |
| POST /api/listings/facets | {listing} | {seq, counts} |
| POST /api/records | {ids}, at most 500 decimal strings | GridRecord binary, with a gone flag per id |
| POST /api/changes | listing entries (section 4.9) | moved listing keys |
| POST /api/selections/apply | section 1.7 | 202 job, or 409 |

Every reply is private, no-store, with brotli or gzip by Accept-Encoding. FileIds in JSON bodies are decimal strings, since an i64 may exceed 2^53.

Reply body: a u32 LE header length, the header JSON, then binary. Header: `mode` (full or delta), `seq`, `catalog_id`, `tombstone_floor`, `count`, `set_digest` (32 hex), `order_digest` (16 hex), `anchor_index` (Full, when an anchor was sent and is present), `left_count` and `changed_count` (Delta). Item: 16 bytes little-endian: i64 file_id, u32 shown w, u32 shown h (Entry::shown_size, grid.rs:74-76, which includes pending turns). Full: count items in final order. Delta: left_count i64 ids, then changed_count entries (u32 final_index, i64 file_id, u32 w, u32 h) sorted by final_index.

### 4.4 Server algorithm

Full: one read transaction. Read seq, bulk_seq, tombstone_floor and catalog_id, run the ordered pass (4.5), build the reply in memory, and end the transaction before writing (no WAL frames stay pinned; temp_store MEMORY, schema.rs:377-378).

Delta(seq, catalog_id), one read transaction:

1. Answer Full if catalog_id differs, seq > meta.seq (an older restored catalog), seq < tombstone_floor, or bulk_seq > seq.
2. Let C be the files with changed > seq (the files_changed index, schema.rs:543).
3. Make one ordered pass over the scope, evaluating the membership condition for every member, computing count, set_digest and order_digest, and recording the final index of each member in C.
4. left = (C minus members) plus file_tombstones rows with seq > the request's seq and order_key in scope. changed = the members in C with final indexes, sorted by index.
5. If |C| > 50,000 or |C| > 10 percent of count, answer Full (P). The bound counts all of C because left is also transfer.

The Delta costs one O(N) pass, the same as Full; transfer is O(changed). Digests are always computed, since the pass visits every member; the verify flag and an every-eighth cadence are removed (kept only as a fallback if hashing is measured to matter).

Order and ties: name order is ORDER BY order_key, then id, both in the sort direction (grid.rs:156-168). Keyed sorts order by order_key ASC, id ASC (grid.rs:164-166), then list_grid applies a stable sort with compare_keys, reversed when descending (grid.rs:113-153, 83-96). Ties are ordered by order_key then id ascending in both directions; NULL keys come last in both. The rank test pins it. The client never sorts.

Membership: scope AND query AND missing = 0 AND unpaired = 0. Scope is the folder's order_key range (scope.rs:11-20); query is the query condition (crates/query/src/lib.rs, with kind and frames as membership columns, lib.rs:93-95); missing = 0 matches the partial index (schema.rs:535); unpaired = 0 (O37). Today list_grid tests only missing (grid.rs:164-166), so provisional rows appear and then vanish after pairing; the change is a desktop change too.

### 4.5 Digests (pg-wire, wasm32-clean)

- mix64 is the splitmix64 finaliser over the u64 bit pattern of an id.
- set_digest: two wrapping u64 sums, of mix64(id ^ K1) and mix64(id ^ K2), giving 128 bits. K1 and K2 are constants in pg-wire.
- order_digest: sum over final index i (from 0) of mix64(id_i ^ mix64(i ^ K3)), mod 2^64.
- The client recomputes count and both digests during its merge pass. A mismatch drops the replica, requests Full, and logs the listing_key and both values.
- Resolved: BLAKE3 per id (slower on wasm, no gain), the polynomial order hash (structural collisions), and a 64-bit set digest for bulk actions are rejected.

### 4.6 Stamps, tombstones and the bulk marker (migration 10)

A membership, order or shape change sets `files.changed` (schema.rs:524) to the new seq. It never sets `files.seq`, the thumbnail stamp (schema.rs:514 comment; files_source_changed at schema.rs:728-734).

- files_membership_changed: AFTER UPDATE OF created, folder_id, name, ext, order_key, unpaired, kind ON files, WHEN a value differs. Sets changed. When folder_id differs, also sets meta.move_seq. take_identity writes these with no stamp (pair.rs:352-363).
- analysis_membership_changed: AFTER UPDATE OF width, height, orientation, taken, frames, duration_ms, error ON analysis. Sets changed for the file. analysis_changed (schema.rs:755-770) is untouched. The analysis INSERT stamp is unverified; the enumeration test decides, and an AFTER INSERT stamp is added if it fails.
- ratings: AFTER INSERT, AFTER UPDATE OF rating, AFTER DELETE set changed (rating and rating_seq stamp). file_writes: AFTER INSERT, AFTER UPDATE OF value, AFTER DELETE set changed for the rotation/pending kind. border_marks: AFTER INSERT, AFTER UPDATE, AFTER DELETE set changed.
- Deletes: AFTER DELETE ON files writes file_tombstones(file_id, seq, at, order_key). Pair merges (pair.rs:345, 434) are covered. A daily prune in a write transaction deletes rows older than 90 days (P) and sets tombstone_floor to the largest pruned seq. Nothing stamps a deleted row, so the tombstone is the only removal signal.
- bulk_seq is set to the seq of any bulk change no row stamp carries: the AUTOINCREMENT rebuild and the taken-zone rewrite. It is named bulk_seq, not a layout epoch, so it is not read as a layout change under the decided hold rule. The Taken key reads localtime at query time (query lib.rs:362-366), so each listing read compares TAKEN_ZONE_QUERY (types.rs:233-236) with meta.taken_zone; a mismatch runs one ordinary write transaction (set_mark and the rewrite, lifecycle.rs:86-92, 164-171), sets bulk_seq, and then the read proceeds. No exclusive hold is taken.
- The membership column list is generated from the query types (SortKey variants, query.rs:124-137; the sort's turn dependence, query.rs:141-145). The generator is not written.
- meta rows bulk_seq, tombstone_floor and move_seq start at 0.

Resolved: stamping files.seq for membership wakes thumbnail work for no reason; a Rust-side touch called by writers misses unwired writers; extending analysis_changed needs a drop and recreate; the per-row membership evaluation is not used.

### 4.7 AUTOINCREMENT rebuild (migration 10)

files becomes `id INTEGER PRIMARY KEY AUTOINCREMENT` (baseline schema.rs:502; no AUTOINCREMENT exists in crates/). The rebuild runs inside prepare before bring_to, on a connection with foreign_keys off: SETTINGS turns foreign keys on (schema.rs:377), child tables cascade on files delete (schema.rs:113, 204, 545, 591, 608), and a migration runs in one transaction (schema.rs:23-26), so the rebuild cannot run as an ordinary migration step. Steps: create files_new with the same columns and constraints (UNIQUE(volume, file_ref), schema.rs:528); copy every row with its id; drop files; rename files_new to files; recreate every index and trigger on files, captured from sqlite_master (including files_folder, files_ref, files_fresh, files_added, files_changed, files_source_changed, files_failures_stale, files_failures_hidden and the new stamp triggers); run foreign_key_check; commit; set bulk_seq. A test compares the files objects in sqlite_master before and after.

The preferred single procedure is the existing rebuild path (rebuild.rs CARRIED copy, rebuild.rs:181-210; not read in this pass). If it carries ids and recreates triggers, use it; otherwise the steps above. A bare DROP with foreign keys on is rejected (it deletes every child row).

### 4.8 Client replica and merge (option A)

- In the layout worker, in memory only. ids as BigInt64Array (FileId is i64, ids.rs:10-11), shapes as Uint32Array with two entries per item. The main thread receives only the visible window. A reload sends Full.
- At most two replicas: the one shown and one recent. The least recently shown is dropped.
- Full replaces the replica with the header's seq and catalog_id.
- Delta: build in a spare buffer; remove ids in left or changed; insert changed items at their final indexes in ascending order; recompute count and both digests; swap in with the new seq only if all match. Otherwise drop the replica and request Full.
- A duplicate reply has the same seq and is a no-op. An interrupted apply leaves the old arrays. Peak memory is about twice 16 bytes per item.
- Pull triggers: a moved entry; reconnect; wake; a catalog change. At most one pull per replica per second (P). A pull is deferred while the viewer is open, a rating is in flight, or the grid is scrolling; it runs on viewer close or 300 ms after scrolling stops (O35).
- Layout: layout::justify_into (layout.rs:38-53) reruns in the worker on resize or shape change. Aspect is f64 width over f64 height from the integers, 1.0 when either is 0 (grid.rs:79-82), clamped by clamp_aspect (layout.rs:16-18). The header's ratio blob (WEB_CLIENT.md:315-316, 393-394) is withdrawn.
- Lifecycle: no grace period, idle timer, cap or budget on the server. Nothing expires there. A replica is dropped on reload, on digest mismatch, or when a third is needed (least recently shown first). A server restart changes nothing: seq, changed, tombstones and bulk_seq are durable. A restored backup is caught by the poll (seq above meta.seq or a catalog_id change) or by the digests.

### 4.9 Change notice under A

The poll carries one entry per held replica (at most two), `{listing: listing_key, seq, catalog_id}`, in POST /api/changes (1.1). The server answers moved when any of these holds:

1. catalog_id differs, seq > meta.seq, seq < tombstone_floor, or bulk_seq > seq;
2. some files row with changed > seq has order_key in the listing's scope (files_changed);
3. meta.move_seq > seq (a folder move anywhere);
4. some file_tombstones row with seq > seq has order_key in the scope.
Rule 2 alone misses a file moved out of the folder, since its new folder is outside the scope; rule 3 covers it. Renames and rating edits keep the folder, so rule 2 covers them. Over-notification is harmless: the Delta comes back empty. One 250 ms window batches. The server keeps no list of open listings.

Under option B the same moves are the listing scopes of section 1.1.

### 4.10 Selection, bulk actions, facets, rating edits

- Selection follows FileId. If the selected id leaves before a Delta is applied, the client selects the next surviving FileId in its old order, or the previous one (O36). The scroll anchor is the selected item's row after the Delta (grid.rs:183). This replaces the clamp at grid.rs:391-395 and the doc rule at doc:216-218.
- Select-all: section 1.7 (count and set_digest verified in one read transaction; collected ids go into the job).
- Explicit ids: each probed against the membership condition in one read transaction; any non-member gives 409 selection-changed.
- Facets: `POST /api/listings/facets` returns counts over current membership with the seq read in the same transaction. The client shows them only when that seq equals its replica's seq; otherwise it pulls first.
- Rating edits: the record updates at once. Under a rating sort the list order waits for the pull gate (O35). The desktop edits in place (crates/desktop/src/gallery/edits.rs:75-92) and reorders on the next reload (Main.qml:255-265); the doc claim at doc:224-226 is withdrawn.
- Held pulls mean scan changes reach the viewer as a notice within 250 ms, and the list update waits for the gate. The decided "live" rule is met by the notice and by the held list update (O35).

### 4.11 Cost, concurrency and failures

- LAN budget: 5,000,000 items served per device per minute (P, about 80 MB); a Delta counts its changed entries only. Loopback is exempt.
- At most two listing builds server-wide (each buffers up to 16 MB at a million items); beyond that 503 busy with Retry-After.
- One read per device per listing_key; a newer request cancels the older through the progress handler (crates/catalog/src/core.rs:173-179), as crates/app/src/reader.rs:209-238 does.
- Failures: server crash or restart changes nothing; the phone sleeping or the connection dropping keeps the replica's seq; a restored backup gives Full or the digest mismatch; a write during a computation appears in that reply or the next; desktop restart or token rotation loses the replica and the first open is Full; a phone killed by Android loses the replica and reopens with Full after viewstate restores the listing; a zone change is caught by the check in 4.6.
- Resolved: the four-listing cap, grace, idle timer and 256 MB budget (ops and wire) are void under A. The 56-byte figure (WEB_CLIENT.md:214) is the desktop's Entry (grid.rs:14-41), not a server cost. Ten Full replies a minute per device (R1) and per-IP limits are rejected.

### 4.12 real_size, the 1:1 control and the web viewer

real_size:

- A bool in the this_computer device's prefs (remembered table), serde default false. Browser devices have none.
- Before cutover it stays in gallery.json (prefs.rs:28-29). The cutover import copies it into the this_computer row once, with the marker in the same server.db transaction. After cutover nothing writes gallery.json; the import is its only reader.
- `PUT /api/settings/real_size {value: bool}` is mounted on the loopback router only, accepted only from the this_computer principal. Absolute, last writer wins, no version (the desktop is the only writer). The LAN router omits the key from GET /api/settings and answers 404 to PUT. Writes are coalesced (one in flight, latest kept). Failure: 507 storage-full; the desktop keeps the value in memory and shows PrefsNotSaved (prefs.rs:147-154).
- Startup gate: the control and Z are inert until the settings read returns. No default is written over the stored value.
- Resolved: a browser column with defaults (device), per-user storage, server-global like write_back, keeping gallery.json authoritative after cutover, and the fixed 'desktop' key are rejected.

The 1:1 control:

- ONE_TO_ONE_MARGIN = 0.01 is a constant in crates/client/src/viewport.rs, separate from the 0.0001 reset guard (viewport.rs:171). Yandex's literal (YandexPreview.qml:169) reads the same constant through a bridge property.
- The button shows when the view is zoomable (viewport.rs:68-70) and |fit_zoom - 1| exceeds the margin. Label: `1:1` when fitted; `ui-fit` text at 1:1 (ui.ftl:179).
- Toggle: toggle_one_to_one (viewport.rs:178-182) at the viewer centre. Double-tap uses the same toggle at the tapped point (Viewer.qml:354-363). State is transient per image: a step calls reset(false) (viewport.rs:169-176). Nothing is persisted or restored after reload.
- Only zoomable stills. Video is not designed.

Keys and hints:

- Z stays in the shared key map (keys.rs:133; Hint::RealSize keys.rs:225). Desktop: Z toggles the persisted value (Viewer.qml:279). Web with a keyboard: Z toggles the transient 1:1 and writes nothing; the hint reads `Z 1:1` (new ui-hint-z-one-to-one).
- viewer_hint (keys.rs:294) takes an Input enum: Desktop | WebKeys | Touch. Desktop and WebKeys differ only in the Z label. The Touch line lists pinch zoom, drag to pan when zoomed, swipe to step or rate when fitted, double-tap 1:1 (new ui-hint-touch-one-to-one), the 1:1 button, and H to hide; it omits wheel zoom, Z, L, R and the rate keys.
- Gestures: pinch around the centroid, clamped between minimum zoom and 32x (Viewer.qml:314-320; viewport.rs:6, 155-167); drag to pan when zoomed (Viewer.qml:332-352); swipe to step or rate when fitted, at 60 px (Viewer.qml:346-347); at 1:1 a horizontal swipe pans (Viewer.qml:340, 349-350). Overscroll and touch-action are checked at the phone gate.

View URLs (option A and the web; cluster 1's key rules):

- `GET /api/images/{catalog_id}/v{VIEW_VERSION}/{file_id}@{mtime}.{rotation}.{pending}/fit/{w}x{h}`
- `GET /api/images/{catalog_id}/v{VIEW_VERSION}/{file_id}@{mtime}.{rotation}.{pending}/tile/{col},{row}`
- `GET /api/images/{catalog_id}/v{VIEW_VERSION}/{file_id}@{mtime}.{rotation}.{pending}/region/{x},{y},{w},{h}/{pw}x{ph}`
- The key segment is the desktop's ImageKey text (crates/desktop/src/gallery/items.rs:41-48), parsed by one shared parser (crates/client/src/image.rs:119-135) under two policies: the desktop parses leniently, the web strictly (region coordinates are decimal fractions with exactly four decimals in 0 to 1; anything else is 400, so each URL has one cache key).
- catalog_id guards rebuilt catalogs; a key that is not current (catalog_id, VIEW_VERSION, mtime, rotation or pending) is 409 stale-key with no-store. VIEW_VERSION is a new constant beside THUMBS_VERSION, bumped on any change to decoding, resampling, encoding or the cache schema.
- pending is part of the bytes: full_image applies unwritten turns (crates/app/src/images.rs:275-279, 290). The tile grid is laid out over the shown image (Entry::shown_size, which includes pending turns); decoded dimensions must equal it, or the reply is 409.
- The full-image form is not a web route. Video gets 415.
- Replaces: wire's /api/items/{file}/view (design:1475), realsize's /api/files/{key}, the doc sketch (doc:395), and /api/views (taken by saved views).

Bounds: the fit and region limits stated once in `WEB_CLIENT.md` (Limits). A larger request is 400 bad-request; the server never clamps. The client parser stops clamping to 16384 (image.rs:134). A DPR-3 portrait phone window (3240 by 7200 device px) is inside the per-side bound and at 23.3 MP. A 64 MP RGBA decode is 256 MB, inside the 512 MiB codec cap (crates/codec/src/codecs/limits.rs:5-10). Transport's 16384 px bound is rejected.

Cache: fit, tile and region 200s use O11's value. Errors and unversioned requests are no-store. Listing and record replies are private, no-store. Auth's no-store on view endpoints (auth:247) is replaced for versioned views. Images load through plain img tags, which need no header (1.2 step 8 exempts them), so the fetch-to-object-URL path is not used.

Cross-site: Sec-Fetch-Site cross-site on an image GET gets 403 before any decode; no cookie gets 401 before any decode (SameSite=Lax, section 2.4).

Server decode:

- The single last_decode slot (crates/app/src/images.rs:92; decode at 289-301) becomes a byte-budgeted LRU keyed by (file_id, mtime, rotation, pending). Each entry records the decode size it was made for (DECODE_MARGIN 4, crates/render/src/display.rs:23, 50-59); a lookup succeeds when the entry covers the view. Budget 256 MiB (P). An entry larger than the budget is held while it serves.
- Single flight per key. One decode at a time (semaphore of 1, P) on a view worker separate from the thumbnail workers. A queue of 8 (P); overflow gets 503 busy with Retry-After. The 512 MiB codec cap bounds one decode.
- Fit and region use the DCT-scaled decode (display.rs:50-59). Tiles need a full decode. Region output resamples through the render crate (display.rs:102-115; the filter is unverified). Tiles are 512 by 512 image pixels, lossless PNG (P). Neighbours are preloaded as fitted images only (Viewer.qml:396-404).

Failures: transient errors (dropped connection, 503, timeout) retry with capped backoff (P); the fitted image stays; an error shows after the cap. Permanent errors (409 stale-key, 404 for an out-of-range tile, 415, corrupt original) are not retried; a 409 refetches the item and drops old tiles. Sleep aborts in-flight tile requests; wake requests visible tiles only. A rotation or turn while zoomed is a new key.

Yandex: the preview keeps its button (YandexPreview.qml:139-144) with the shared threshold and its single-tap toggle (:181-183). On the web in v1 the button is hidden (O32). Its only source is the host URL (YandexPreview.qml:172), outside the version rule; a server fetch of result URLs is the SSRF surface wire closes.

Resolved in 4.12: 16384 px and the full-image route (transport) are rejected. The 4096 px LAN cap (listing's first draft) is refuted by the phone case. Realsize's region no-cache is dropped because the URL names the region. Lan's no-store for views is replaced by O11. Auth's SameSite Strict and the Sec-Fetch-Site refusal are resolved in 2.4 and 1.2.

### 4.13 Could not verify (cluster 4)

- The analysis INSERT stamp; cascades from files deletes into stamped tables (schema.rs:113, 204); rowid reuse without AUTOINCREMENT (taken from the SQLite rule, not reproduced); set_mark's body (lifecycle.rs:86-92); the four reference instants of the taken-zone fingerprint (types.rs:233-236, not measured on Windows).
- The migration runner's transaction shape and whether foreign_keys can be switched off around the rebuild; the rebuild path (rebuild.rs:181-210) was not read.
- The Resizer filter used by fit_box and resize_region (display.rs:107, 111); the consistent() busy-wait on the view path (images.rs:30, 141-160); whether decoded dimensions equal the shown size for every format and EXIF orientation (items.rs:94-101).
- The viewer bridge's property mechanism for sharing ONE_TO_ONE_MARGIN with the Yandex QML.
- Qt for WebAssembly: an object URL on a QML Image; touch via PinchHandler and TapHandler tapCount; overscroll and touch-action on Chrome Android; whether CSS image-rendering pixelated matches Qt smooth=false above 1:1.
- Clear-Site-Data on Android Chrome, and whether the HTTP cache honours immutable after it; Chrome's handling of a 200 image with the private directive.
- Fit encoding (PNG or JPEG) for the fitted image is decided by neither design; measure encode time and bytes at 2560 by 1440. Tile pixel identity with the desktop's 1:1 render is a test to write; not run.
- Region coordinates with pending turns applied are assumed, not checked against Viewer.qml's requests.
- Measurements, none run: the Delta pass at 189,518 and 1,000,000 items; the Full read and transfer; the digest hashing cost; the notice probe's EXISTS over changed rows; select-all memory (about 8 MB at 1,000,000 ids); replica memory and merge time on Android Chrome; decode time per format for a 24 MP still; PNG encode time and bytes per tile; first-pixel time for 1:1 on a phone; the decode-cache budget; the 1% margin; the 8192 px and 64 MP bounds; the 512 px tile size; the 250 ms and 1 s floors under a real scan's bump rate.
- Every number here marked (P) is a proposal.

---

## 5. TLS, the LAN listener and operations

### 5.1 Placement

- home (`<home>`): server.db; launch.json; gallery.json (rollback mirror); gallery.log; desktop.lock; home.lock (section 2.1); tls/lan-ca.bundle (intermediate certificate and its DPAPI-protected key); tls/root.cer (public root).
- catalog_dir (`<catalog_dir>`): catalog.sqlite; server.lock.
- The root private key: an owner-chosen file outside both folders. The server never keeps it at rest.
- Nothing is configured by path except the certificate paths in server.db, which name the fixed files above.
- Resolved: paths in gallery.json (auth) are rejected; configured paths (lan's first draft) are rejected for the fixed names; everything in catalog_dir (puts key material and credentials in backup scope) is rejected; everything in home (the lock would not guard the catalog folder) is rejected. The names server.sqlite, auth.sqlite and serve.json are withdrawn; launch.json in home is the one launch file (B9).

### 5.2 Server start (combined order)

1. Try server.lock exclusive. If held: a matching server answers hello (exit 0), anything else exits 75.
2. Open server.db (WAL); read the LAN settings.
3. Judge server.db (section 2.2).
4. Remove a leftover `.building` (rebuild.rs:67-68) at every start.
5. Catalog::prepare (section 3.2), with the exclusive hold only in the change branch (core.rs:91-92). The server keeps the catalog's shared hold for life (core.rs:124-125).
6. Write launch.json (owner-only DACL; temp, FlushFileBuffers, rename).
7. Bind loopback and serve it.
8. Start the LAN listener only if the checklist in 5.11 passes.

A failure at steps 1 to 7 ends the process. A LAN failure never stops loopback or the catalog. Exit codes: 0, 75, 1 (section 3.2).

### 5.3 The supervisor

- One per-user scheduled task, `gallery-serve`, created from an XML definition the desktop writes with `schtasks /Create`. Action: `gallery serve --home "<home>" --catalog-dir "<catalog_dir>"`. Logon trigger per O5. Restart on failure 3 times at 1-minute intervals (P). MultipleInstances IgnoreNew; ExecutionTimeLimit PT0S; battery allowed.
- The task exists from the first server-enabled build, whether or not the LAN is on, because the desktop needs the server for every read after cutover.
- The desktop starts the server only by `schtasks /Run`. It never spawns a server, never kills a process, and never restarts the server itself. It shows the last task result and offers to run the task again.
- Resolved: the desktop-spawned detached process (writes, cutover) is rejected: the kill-on-close job (process.rs:104-115) would couple lifetimes, and two supervisors would fight. A Windows service needs administrator rights and a different profile and DPAPI scope. A Run key or Startup shortcut has no restart and no history.
- Unverified: whether creating a per-user task needs elevation. If it does, the fallback is the O5 startup-entry option; Direct (O23) is the only route back.

### 5.4 Stop, crash and logoff

Stop is `POST /api/server/stop` with the bearer (build-exempt, loopback only). Order: stop accepting on both listeners; answer held polls with 'stopping'; close sessions; stop batch jobs at a file boundary; drain within bounds; write the mirror and the catalog session row; close the pool; drop the catalog keeper last; delete launch.json; release server.lock last. Crash, logoff and shutdown are one case. No design depends on a graceful stop; a mutation is acknowledged only after its commit.

### 5.5 Pre-login surface and guards

- Pre-login: `GET /login` and `POST /api/auth/login` only (1.2 step 5). Every other request without a session gets 401 with one JSON body, or a redirect to /login for a document request.
- /login is one HTML document with inline CSS and script, each covered by a CSP hash, and an inline data: icon so no favicon is requested. It has no other subresource and carries bundle_id. Its CSP: default-src 'none'; script-src and style-src by hash; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'. The app's own CSP is separate (5.15).
- Removed from the pre-login set: the favicon route and its 204, the events handshake GET, pre-login hashed assets, and the notices file (GET /notices, behind login).
- Guards are the cluster-1 gate. Unsafe requests need Origin, application/json and X-Pg-Client; a build header is required on build-gated routes. Lan's X-Gallery-Build (one header for CSRF and build) is rejected in favour of the two-header rule. Its exemption list (hello, stop, media and bundle GETs) is kept.

### 5.6 Login admission

Admission numbers are in section 2.11: 5 failures, a 30 s doubling wait to 15 minutes, a 1 s global gap after a failed verification, one verification thread with a queue of 4, and 429 with Retry-After. The persisted counters are in server.db. A peer-level connection cap (5.10) applies before TLS.

### 5.7 TLS stack

- rustls 0.23 with the ring provider named in code, through tokio-rustls 0.26 for the LAN listener. TLS 1.2 and 1.3 only; no client authentication. ServerConfig's certificate resolver is backed by an atomic swap so the leaf can change. ALPN offers h2 and http/1.1; dispatch follows the negotiated ALPN; no plaintext and no redirect on the LAN port. The chain served is [leaf, intermediate]. Handshake timeout 10 s (P).
- The key-match check runs when the certified key is built; the API names are unverified.
- ureq 3 (Yandex and the desktop's HTTP): default features off, with rustls-no-provider, platform-verifier and cookies (the `ureq` entry in `[workspace.dependencies]` uses native-tls today). The provider is passed explicitly to ureq and to ServerConfig; no process-wide default is installed.
- native-tls, openssl and openssl-sys are banned in deny.toml (the `[bans]` deny list in deny.toml gains the bans).
- Corrections: ureq already uses native-tls (WEB_CLIENT.md:384; the `native-tls` package in Cargo.lock), so the change moves it to rustls with one explicit provider; it does not reuse an existing provider. rustls is pure Rust only for the crate; ring carries C and assembly. WEB_CLIENT.md:380-387 says native-tls has no server ALPN; its schannel accept path exists behind alpn-accept (schannel.rs:334-375), which this build does not enable. The reasons against native-tls still stand: a blocking acceptor, the chain FIXME at schannel.rs:360, and a per-process CryptoAPI container.
- The server negotiates TLS 1.2 and 1.3 only; the ring provider covers both.
- Rejected: two TLS stacks (two trust policies, OpenSSL stays in CI); native-tls for the listener; aws-lc-rs (needs NASM and gives nothing while ureq uses ring); a reverse proxy; axum-server.

### 5.8 Yandex TLS (its own commit, first)

- The Yandex change ships before the LAN code, as its own commit: the `ureq` entry and the workspace rustls entry (ring, std, tls12, logging); crates/yandex/src/http.rs with one constructor serving client.rs:38 and fetch.rs:29 (the comment at http.rs:26-29 rewritten, and a cfg(test) read-back test asserting PlatformVerifier and a Some provider); deny.toml; Cargo.lock; libssl-dev removed from docker/apt-packages.txt:11, tools/profiling/ci_setup.py:36 and docs/reference/PROFILING.md:218, only after Linux CI passes without it.
- Behaviour changes, gated by O33 and a paired live measurement: online end-certificate revocation with soft-fail on unknown status; a connect-time wait of up to about 10 s per new end certificate; Linux roots from rustls-native-certs; changed error text.
- Checks: `cargo tree -i` for native-tls, openssl, openssl-sys and schannel (none present) and ring (rustls only); cargo deny; Linux CI without libssl-dev; a manual root check with a test CA; paired live searches and downloads on old and new builds (about 50 per build, P).

### 5.9 Private CA

Three levels (O44):

- Root: ECDSA P-256, 10 years, pathLen 1. Created in memory at setup and used to sign the intermediate. Its key is written once to the owner's file outside both folders, read back, compared by SHA-256, then dropped. The server keeps no root key at rest. Renewal, and any new intermediate, need the owner to supply that file: the desktop reads it, checks it against root.cer, signs, commits the bundle, and drops the key.
- Intermediate: ECDSA P-256, 5 years, pathLen 0, with name constraints: permitted IPv4 is the adapter's network at signing; permitted DNS is the stable LAN name. Stored in lan-ca.bundle.
- Leaf: ECDSA P-256, 90 days, SANs for the stable name and the current IPv4, backdated 5 minutes. Issued at each start, renewal and address change. Held in memory only and never written.
- The phone trusts only the root.

Setup order (crash-safe): LAN disabled; the owner picks the adapter, port and stable name, with the owner password already set. Generate the root key in memory; write it to the owner's file by temp and rename; read it back and compare SHA-256. Write root.cer (temp, FlushFileBuffers, rename). Sign the intermediate; write lan-ca.bundle (temp, FlushFileBuffers, rename): this rename is the CA commit. Drop the root key. Write the LAN settings to server.db. Show the root fingerprint. A crash before the bundle rename grants no trust; orphaned root key and root.cer files are named in a notice for deletion.

lan-ca.bundle: format version (u32), created_ms, root SHA-256, the intermediate DER, and the PKCS#8 key protected with DPAPI at user scope, written by temp, FlushFileBuffers and rename, with an owner-only DACL at creation. DPAPI is mandatory: if it is unavailable, enabling the LAN fails with a message and no plaintext key is written. A bundle that fails to parse, unseal or match its certificate stops the LAN listener with a notice.

Renewal: the leaf is checked hourly, on resume from sleep, and on each address change; it is reissued when fewer than 30 days remain or its SANs differ. The new leaf is swapped into the resolver; open connections keep theirs. A failed renewal keeps the old leaf to its expiry, logs, and raises a notice at once. At expiry handshakes fail and LAN access stops; there is no HTTP fallback, and loopback and the catalog keep running.

Notices (new Notice variants, around notice.rs:352): renewal failure at once; leaf not renewed, 14 and 3 days before expiry; intermediate 90 days before expiry; root 1 year before.

Phone trust: the desktop exports root.cer (DER or PEM) and shows its SHA-256. The owner moves the file to the phone by a channel of their choice and compares the fingerprint at install. The root is never served on the LAN. This is a one-time trust step, not pairing; no QR code. Rejected: serving the root over the LAN; mkcert (writes the Windows root store; Qt's dev server uses it at qtwasmserver.py:55-61); CertCreateSelfSignCertificate (persistent containers).

### 5.10 LAN bind, firewall, address change, limits

- The listener binds the IPv4 address of one adapter the owner selects by its GUID (stored in server.db), on the owner's port, never 0.0.0.0. IPv4 only in v1; IPv6 is open, not rejected. The default-route adapter is pre-selected only if its network is Private. If the adapter loses its IPv4 address, the listener stops with a notice and never moves.
- A peer outside the adapter's IPv4 prefix is closed before TLS.
- Address change: NotifyIpInterfaceChange plus a 30 s poll as backstop (P). On change, the server issues a new leaf with the new SAN set, binds a new socket on the new address (make before break), swaps the resolver, and closes the old socket. A move outside the intermediate's name-constraint prefix raises a notice.
- Firewall: the rule requires the Private profile, RemoteAddress LocalSubnet, and this program and port. A Public network gets no rule and LAN access stays off with a notice.
- LAN is off by default (O43). The phone addresses the server by the stable name; cookies have no Domain, so they bind to the host the phone used.
- Caps (P): 64 connections total and 8 per peer IPv4 after login; 16 total and 4 per peer before login. Over-cap connections are closed at TCP level, not queued. A pre-login connection with no completed request in 30 s is closed (P).
- Resolved: wildcard bind with a peer filter, any RFC1918 address as LAN, re-pairing on address change (none exists), an IP-only SAN (breaks on lease changes), and mDNS as the only discovery (.local support unverified) are rejected.

### 5.11 LAN start checklist

The LAN listener starts only if every check passes, in this order: LAN enabled in server.db; owner password present; lan-ca.bundle parses, unseals, matches its certificate and is unexpired; the root SHA-256 in the bundle equals root.cer; the adapter has an IPv4 address inside the permitted prefix; the Private profile and firewall rule are present; the web bundle manifest verifies (5.15); a leaf is issued. Any failure leaves the LAN down with a notice. Loopback and the catalog always start.

### 5.12 Timers, body limits and HTTP/2

- Timers (P): TLS handshake 10 s; request headers complete in 10 s; a pre-login connection without a completed request in 30 s; keep-alive idle 60 s. A response body that moves no bytes for 30 s is cancelled; the timer starts at the first response byte, so a held poll is not cut.
- Body limits: section 1.10. Reads use the cap plus one byte. Request headers 16 KiB; HTTP/2 header list 16 KiB. The framework default is disabled.
- HTTP/2 (P): SETTINGS_MAX_CONCURRENT_STREAMS 128, sized for a screen of thumbnails (1.5); a connection that resets more than 100 streams in 10 s is closed. The hyper and h2 versions must carry the CVE-2023-44487 fixes; that is a gate on adding them. The reset rate is measured during fast scroll.
- HTTP/2 negotiates only where TLS does (the TLS cluster's question; section 5.7). The stream cap needs HTTP/2.

Resolved: lan's values (32 streams, 200 resets) and ops' 100-stream cap are rejected in favour of the thumbnail-sized values, pending the fast-scroll measurement. The HTTP/1.1-only option is rejected (loses multiplexing).

### 5.13 Durability and disk

- The mutation connection is a dedicated catalog connection with PRAGMA synchronous = FULL. It carries user mutations, their ledger rows, and writes to server.db's remembered and credential tables that are acknowledged to a user (login success, logout, revoke, setting writes). A mutation is acknowledged only after its commit.
- login_failures uses NORMAL (a lost last count on power loss is accepted). Batch connections (scan batches, thumbnail writes, rebuild staging) keep NORMAL from SETTINGS (schema.rs:377). `synchronous` is per connection, so batches never use the mutation connection.
- FULL ships only after the owner's disks show acceptable commit latency (measured).
- Free space: a new free-bytes function (volume.rs:17 passes null and reads only the total), checked every 60 s and before large jobs, on the catalog volume, the thumbnail-cache volume, home, and the originals' volume for write_back. Background writes stop below max(1 GiB, 2% of the volume) free. User mutations stop below 256 MiB free (P). A mutation that cannot be stored answers 507 storage-full; login and revoke answer 503 with Retry-After.
- Thumbnail cache cap: the lesser of its configured cap and a free-space value. Today's default (total/20 clamped to 1 to 50 GB, thumbcache config.rs:23-35) reads only the total.
- Migration backup removes its .partial on failure (rebuild.rs:43-47, which leaves it today) and is refused when free space is below the catalog size plus the floor.

### 5.14 Threads, I/O priority, sleep

- Request handlers call pg-app on the tokio blocking pool, sized to the catalog connections (decided). Batch workers are dedicated std threads that take a connection permit from the same budget.
- FileIoPriorityHintInfo through SetFileInformationByHandle, on by default (decided), on background handles only: scan open_read (pipeline.rs:288 through file.rs:12-30), hashing (analyze.rs:40), and batch thumbnail originals (maker.rs:86). Visible misses, viewer reads and cache serving keep normal priority. If Windows refuses the hint on a regular handle, the design runs on CPU priority alone and logs it. Measurement tunes the hint; it does not gate it.
- Batch threads set CPU priority at start (priority.rs, as pipeline.rs:131 and 166 do). PROCESS_MODE_BACKGROUND (measured slower, THUMBNAILS.md:78-82) and THREAD_MODE_BACKGROUND_BEGIN (unmeasured) are not used.
- Sleep: the server does not keep the PC awake by default. Each job thread holds system-required execution state through SetThreadExecutionState while it runs (per-thread lifetime unverified). A permanent keep-awake setting exists, off by default (O42). Sleep stops the server and the LAN listener; the phone gets no answer until wake.

### 5.15 Bundle serving and notices

- Bundle files carry content-hashed names. A manifest lists each name with its BLAKE3 hash; bundle_id covers it (1.3). The manifest is checked at start; a failure leaves the LAN down with a notice and loopback and the catalog keep running. Bundle files are served only behind the session, except the login page.
- Each file is written at build time as .br and .gz beside the original. The server selects by Accept-Encoding, sends Vary: Accept-Encoding (whether ServeDir sets it is unverified), and streams from disk. Nothing is compressed per request. .wasm is application/wasm.
- Headers: documents carry Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp. Every response carries Cross-Origin-Resource-Policy: same-origin, X-Content-Type-Options: nosniff and Referrer-Policy: no-referrer; no HSTS. index.html and the login page are no-cache with an ETag. Hashed bundle files are private, max-age=31536000, immutable. API JSON is no-store.
- The app CSP target is default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none', adopted only after the Qt loader's needs are checked (unverified).
- Notices: the build generates the third-party notices from the Qt kit's SBOM, the Emscripten licence (MIT), the Rust licences from cargo-deny, and the qtloader.js licence line (qtloader.js:2). Until the owner decides the Qt route (O45), the notice lists both options and says the choice is pending. It is served at GET /notices behind login; the desktop reads the same file locally.
- Emscripten 5.0.5 is pinned in the Justfile beside the FFmpeg pin. tools/doctor.py has no emsdk check today; a check that reads the version of the emsdk the build uses is proposed. Qt's configure check is the second guard.

### 5.16 Logging

- The access log records method, route template (not the expanded path), status, latency and peer IPv4. It never records Cookie, Set-Cookie, Authorization, the loopback token, login bodies, query strings or request bodies. One redaction function enforces this.
- Auth failures, rate-limit trips, cap hits, and rejected Host or Origin values (truncated to 128 characters) are logged with the peer address to gallery.log.
- Panic messages carry no request data. The desktop panic hook (panic_log.rs:1-10) follows the same rule. Rotation: pg-log's capped rotation, proposal 5 MiB by 3 files (cited, not re-read).

### 5.17 Rollback and migrations

- Once any server has run on a catalog, the pre-cutover build is not a rollback: it takes no server.lock and reads no run files, so running it against that catalog is unsafe. It is also refused after migration 10 (core.rs:75-80).
- The rollback lever is the in-band Direct backend in the cutover release (O23). It runs the pre-cutover path under server.lock shared and refuses to start while a server holds the lock exclusively.
- No server-ran marker is written; an older binary would not read it. The lock and the refusal at core.rs:75-80 are the only checks that bind an older build.
- Migration timing: O17. Once migration 10 ships, rollback to the pre-migration build ends (O18).

### 5.18 Build identity

build_id is defined in section 1.3 (source-based, over the workspace sources, manifests and Cargo.lock). Build equality is the only compatibility test besides the envelope integer in the hello reply. The events design must confirm the input set.

### 5.19 Resolved conflicts (cluster 5)

- Exclusive hold: the decided rule governs. The unconditional probe at core.rs:92 moves into the change branch; the server keeps its shared hold; the cutover's startup probe and auth's probe on auth.sqlite are withdrawn (2.1, 3.2).
- CLI writes through the API while the server runs (3.8); the WSL refusal (O6) is the one kept refusal and is named as a tension.
- Desktop prepare after cutover: the desktop opens no catalog and never calls prepare; the server runs it (app.rs:62 and library lib.rs:18-19 are removed from the desktop path).
- Login before any other request: the pre-login set is GET /login and the login POST only (1.2).
- Logon: the decided login-while-closed rule needs the task (O5); lan's trigger-tied-to-LAN default is an amendment, not the decided text.
- Single writer: the cutover's startup exclusive probe and the desktop's prepare are removed.
- One-step cutover: section 3.1.
- Thumbnail GET: the image GET never queues (1.5).
- Device and session routes: `/api/auth/*`, /api/devices/*, /api/settings/*.
- Thread model and the snapshot budget: the 256 MB budget is void under option A; the 16-byte header is the sizing if option B is chosen.

### 5.20 Could not verify (cluster 5)

- rustls 0.23 and tokio-rustls 0.26 API names: ResolvesServerCert and CertifiedKey key-match at build; the ALPN accessor; the ring provider export path; unversioned_rustls_crypto_provider stability in ureq 3.5.1.
- The ring MSVC build on windows-2025; rcgen's licence and deny.toml compliance.
- axum, hyper, h2 and tokio are not in Cargo.lock: the CVE-2023-44487 fixed versions, hyper's stream default and the blocking-pool default (512, from memory) are unchecked.
- windows-sys features at 0.61: Win32_Security_Authorization (absent from the `windows-sys` entry), Win32_Security_Cryptography (DPAPI), the IP Helper feature for NotifyIpInterfaceChange and GetAdaptersAddresses, and Win32_System_Power for SetThreadExecutionState.
- DPAPI and ACL calls: CryptProtectData round trip at user scope; SetSecurityInfo owner-only DACL on the bundle, the token and the lan folder.
- Task Scheduler: per-user task creation from XML without administrator rights (if it needs elevation, the O5 fallback applies and Direct is the only route back); restart-on-failure applies only to non-zero exits; logoff behaviour; the default execution time limit; behaviour across sleep and hibernate.
- Firewall: whether creating an inbound rule needs elevation; whether a new network defaults to Public (believed).
- SetThreadExecutionState: whether the state persists when the job thread is the only holder.
- FileIoPriorityHintInfo on a regular file handle (NTFS_MFT_SCAN_PLAN.md:142-143 records a volume handle only); whether THREAD_MODE_BACKGROUND_BEGIN lowers disk time.
- SQLite: whether synchronous FULL in WAL syncs earlier NORMAL commits (believed); durability on the owner's disks, including write caches that lie about flushes.
- Android Chrome (the phone trust gate): acceptance of a user-installed root for the stable name and the IP; enforcement of name constraints on the intermediate; acceptance of a 90-day leaf; Origin and Sec-Fetch-Site on same-origin POSTs and media requests; a `__Host-` Secure cookie on an IP host.
- Network: whether the owner's router resolves the stable LAN name; whether Android Chrome resolves .local names; NotifyIpInterfaceChange and its 30 s poll; a laptop moved to a different prefix (proposal only).
- hyper-util and tower-http: header_read_timeout, max_concurrent_streams and keep_alive_timeout as cited; ServeDir precompressed Vary: Accept-Encoding; the rapid-reset-fixed hyper and h2 versions.
- Argon2id time and peak memory at 128 MiB, t=3, p=1 and at the OWASP floor: not measured here (0.59 s is recorded in `WEB_CLIENT_TECH_DECISIONS.md:147`).
- Qt and the bundle: whether the Qt loader needs unsafe-eval or a relaxed CSP; the Qt WASM secure-context rule for the desktop's 127.0.0.1 pre-screen; whether serving the bundle to the owner's own phone counts as distribution; GPL FFmpeg obligations for the server (crates/README.md:218-226, PLAN.md:327).
- Desktop code: the panic hook at panic_log.rs:1-10; the pg-log rotation parameters (cited, not re-read); whether the desktop's own loopback client sends an Origin (assumed not); the kill-on-close claim was read on the flag (process.rs:104-115), not on every spawn path.
- Yandex: whether every result host validates under rustls-platform-verifier as under schannel; whether the accumulated revocation timeout stays inside the 30 s whole-request bound; the rustls-platform-verifier-android 0.2.0 crate is absent from the local cache.
- Doc claims: WEB_CLIENT.md:325-326 is cited as a native-tls claim by one lens and disclaimed by the validators; left unchanged. The "decided item 31" cited for server.db attached to the catalog is not found in WEB_CLIENT.md, which has no numbered items; this design does not depend on it. The login-first rule is cited at 271-272, 274-275 and 277-281 by different designs; the doc has it at 272-273 and 274-275.
- Cross-cluster: the events design must withdraw its frozen-route Origin exemption, its build-id input rule, public thumbnail caching, its SameSite and CSRF text, and the WebSocket rules if O27 is approved. Its approval is an owner decision.
- Nothing in this cluster was built, run or measured. Every byte size is arithmetic on field widths, labelled as such.

---

## Cross-cluster changes this design requires of other designs

- Auth and device: adopt the two-cookie rule (2.4), SameSite=Lax, `/api/auth/*` routes, the store in home, the desktop key, the narrow prune, and the loopback hello with proof. Drop `__Host-pg_sign`, pg_device, /api/login, /api/me, launch.json and X-Gallery-Request.
- Events: drop the frozen-route Origin exemption for the login POST, the build-id input rule in favour of section 1.3, public caching, and the WebSocket rules (if O27 is approved).
- Wire: retire X-Gallery-Request and the reserved item routes; use request_id in place of op; use the selection caps stated in `WEB_CLIENT.md` (Limits); accept the cache-control value chosen under O11.
- Ops: the 16 KiB JSON cap and the 256 MB budget are superseded by section 1.10 and option A; the four-per-client cap is void under A.
- Cutover: server.lock semantics are section 3.2; the startup exclusive probe is removed; the attach rule is envelope 1 plus build_id plus proof; migration 10 timing is O17; the logon task ships in C8 (O5); gallery.json is a mirror.
- Thumbs: the cache-identity rule, the want-list queue, the 1x1 placeholder, and the lane-priority queue (1.5).
- Realsize: real_size is this_computer only (4.12); the view URLs and bounds are section 4.12; the sign-in cookie is Lax.
- Snapshot: the replica is option A (O16); the budget follows the owner's choice.
- Viewstate: ViewState.listing is the spec with no listing id or generation; the save rule and flush are section 3.9.
