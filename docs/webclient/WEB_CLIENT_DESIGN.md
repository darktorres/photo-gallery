# Web client: area designs

Output of the design workflow (`wf_85df27d2-77d`): twelve areas, each designed by three lenses, judged, and checked by batched validators. This file is **not reconciled**: the validators found 82 contradictions across areas, and some contradict decided items. The contradictions are listed under each area's "Validation notes". Decisions in `WEB_CLIENT.md` govern until this file is reconciled.

## Decisions for the owner

- Approve unifying the TLS stack under rustls: ureq moves to rustls with the platform verifier, and native-tls, openssl and openssl-sys leave the graph (tls).
- Accept the Yandex revocation change: end-certificate online checks, cache-only becomes online with soft-fail on unknown status, and a possible connect-time wait of about 10 s, gated by the live measurement (tls).
- Confirm loopback stays plain HTTP with the per-start token and no TLS (tls, auth).
- Approve the private CA as three levels (offline root, intermediate with name constraints, 90-day leaf), with the root key kept off the server and the root file supplied by the owner for intermediate re-signing (ops).
- Choose the Qt licence route for the bundle (LGPL-3.0-only or Qt commercial) and decide the repo LICENSE file (ops).
- Confirm the LAN is off by default and that enabling it requires a Private network profile (ops).
- Confirm the login policy: no account lockout, per-address backoff only, 15-character minimum password, Argon2id at 128 MiB, t=3, p=1, made the default in this pass (the owner may revisit; `WEB_CLIENT_TECH_DECISIONS.md` O3) (auth, ops).
- Confirm the stolen-phone position: revoke ends one sign-in, a password change ends every browser sign-in, and there is no second factor or recovery code (auth, device).
- Decide whether revoking another device and changing write_back from a phone need step-up (password re-entry). Auth says yes for write_back and revoke; device says no (auth, device).
- Confirm the device model: the desktop is device this_computer with a fixed random id, browsers are one device per profile, and devices are never auto-pruned (device, auth).
- Confirm whether the last IP address is stored on the device record (auth: not in v1).
- Confirm real_size is per device and that yandex_folder moves from per-device to owner-wide (realsize, device).
- Confirm the 1:1 button is shown on the web viewer when a still is zoomable and differs from fit (realsize).
- Confirm that a rating does not reorder the open list at once, and that a new file appears only when its scan's pairing ends (snapshot).
- Approve the client-held listing replica and the server holding no per-listing state; this changes the decided listing stamp from (listing_id, generation) to (listing key, S) (snapshot).
- Approve the selection rule after a removal (next surviving FileId, not the clamped index), which changes desktop behaviour (snapshot, cutover).
- Choose the desktop memory form after cutover: keep the 56-byte Entry, or move to FileIds and shapes (snapshot).
- Approve the change vector and long-poll in place of the sequenced event stream and WebSocket, which amends WEB_CLIENT.md:322-323 and 403-404 (events).
- Approve build equality as the gate on every request (409 build-changed), so a stale page reloads rather than writes (events).
- Decide whether the desktop's thumbnail heights stay unquantized (cache-keyed) or the server quantizes them for every client (thumbs, cutover).
- Confirm the thumbnail refinements: 307 to the nearest kept size, transparent placeholder under no-store, and 503 with Retry-After for overload (thumbs).
- Decide the thumbnail cache-control: private for all authenticated thumbnails versus public (wire and events disagree with thumbs and auth) (thumbs, wire, events).
- Amend the decided mutation rule: turns are server commands with request ids (relative delta), not client absolute values with expected_version (cutover, wire, viewstate).
- Decide whether rating and border-mark writes carry expected_version or stay unconditional last-writer-wins (cutover, viewstate).
- Confirm the atomic ledger write (server.db attached to the catalog connection, one transaction) holds in this build; if not, use the at-most-once turn fallback (cutover).
- Decide whether the in-band Direct desktop backend ships for one release as a rollback lever (cutover).
- While the desktop is still in-process (before the flip), refuse or allow CLI writes when an in-process desktop holds the catalog (cutover).
- Confirm refusing a second desktop process from the start of the cutover (the hazard is two processes, not two windows) (cutover).
- Decide when the per-item mutation version migration ships, since it changes the schema (cutover, viewstate).
- Logon start is decided (O5, `WEB_CLIENT_TECH_DECISIONS.md`): a scheduled task with an always-on logon trigger and 3 restarts at 1 minute. Confirm the task's creation needs no elevation (ops, cutover).
- Confirm the sleep policy: no permanent keep-awake by default, keep awake only during batch work (ops).
- Decide whether the scanner keeps the rating when a file is replaced (pair.rs:10), a scanner-owner decision (wire).
- Confirm the viewer restore policy: viewer and modal panels are not restored after a reload in version 1 (viewstate).
- Confirm that a Start override (--query, --sort and so on) never reaches the saved session; the current code saves it on the first selection change (viewstate).
- Confirm that up to 1 second of last-file and session changes may be lost when Android kills the page (viewstate).
- Confirm the session listing is shared across the user's devices or per device (viewstate, device open).
- Confirm the Yandex result folder and whether a phone may save Yandex results in version 1 (cutover, wire).
- Decide the Argon2id cost after measuring on the owner's desktop, and the session lifetimes (auth).
- Confirm the scope of the Chrome Android cookie and CSRF behaviour before shipping (auth, events).
- Choose whether a Yandex 1:1 source is in scope (realsize: open, not designed).
- Video stays open and is not designed in any area of this document.

---

### TLS library and unification

**Q: What is ureq used for, and is any other crate HTTP or TLS?**
Decision: ureq 3 serves only crates/yandex (pg-yandex): the photo upload POST, paced search GETs with a memory-only cookie jar, and result-file downloads from any host. crates/app reaches it only through pg-yandex. ureq stays for version 1.
Why: Verified by reading the manifests and call sites; the Yandex worker is blocking, paced and holds its own cookie jar, the shape ureq is built for. Reqwest was rejected because its timeout model differs from the per-phase model in use.
Evidence: the `ureq` entry in `[workspace.dependencies]`; crates/yandex/src/client.rs:48-52; crates/yandex/src/fetch.rs:17-19, 35-38; the `ureq` line in the `[bans]` wrapper list of `deny.toml`.

**Q: Why is Yandex set to NativeTls with PlatformVerifier, and what does the http.rs comment say?**
Decision: Two separate things. The provider NativeTls follows from the feature set (the `ureq` entry in `[workspace.dependencies]` enables only its native-tls and cookies features). The policy that matters is RootCerts::PlatformVerifier: trust is the Windows system store. The comment at http.rs:26-29 is accurate. WebPki, ureq's default, uses the bundled Mozilla list with the system store off. The quoted error text is native-tls's own custom-root check, which runs only when built-in roots are disabled (schannel.rs:300-321). The i.ytimg failure was a root-set policy failure inside native-tls, not a library bug.
Why: The fix is the root policy. Under rustls the same mismatch would surface as an unknown-issuer error rather than this text (inference, not run).

**Q: Unify under rustls, keep both stacks, or unify under native-tls?**
Decision: Unify under rustls. One TLS stack serves the Yandex client (ureq with rustls and platform-verifier features) and the LAN listener (rustls ServerConfig behind tokio-rustls 0.26). native-tls, openssl and openssl-sys leave the graph on every platform, and deny.toml bans them. Windows keeps the system store as its root source through rustls-platform-verifier.
Why: OpenSSL leaves the Linux build; native-tls's acceptor is blocking, has a chain FIXME and builds RSA-only identities; one trust and key policy for the process; the Windows chain engine is the same CAPI path.
Rejected: keeping both stacks (two trust behaviours, OpenSSL stays in CI); native-tls for both (blocking acceptor, chain FIXME); rustls with WebPki roots (reproduces the recorded failure).

**Q: Which crypto provider, and what does the Windows build need?**
Decision: ring, selected explicitly in code. The workspace rustls entry enables the ring feature; the agent constructor passes rustls::crypto::ring::default_provider() to ureq; the server passes the same provider to its ServerConfig. No process-wide default provider is installed. The MSVC build needs only the C compiler, since ring 0.17.14 ships pregenerated objects when there is no .git (design read, not built here). No new host tool beyond CMake and NASM already needed for turbojpeg.
Why: One provider, the one ureq's rustls feature would choose anyway. aws-lc-rs needs NASM on Windows and gives benefits this product does not need. Reversal trigger: a post-quantum or FIPS requirement.

**Q: How does Yandex's client verify certificates after the change?**
Decision: Platform verifier, no bundled root list. The agent uses ureq features rustls-no-provider, platform-verifier and cookies (not the rustls feature), sets TlsProvider::Rustls, the explicit ring provider via unversioned_rustls_crypto_provider, and RootCerts::PlatformVerifier. One constructor in crates/yandex/src/http.rs serves client.rs:38 and fetch.rs:29. The comment at http.rs:26-29 is rewritten to name the mechanism correctly.
Why: The provider and verifier stay in one tested place; the unused Mozilla bundle and its licence leave the graph. The unversioned API means every rustls bump needs a check.
Rejected: the rustls feature (dead bundled roots); WebPki (the recorded failure); a hand-written verifier over rustls-native-certs (custom verification code this repo should not own).

**Q: How is the root setting tested?**
Decision: A cfg(test) unit test builds the agent with agent() and reads back tls_config().root_certs(), which must be PlatformVerifier, and the provider, which must be Some. A manual Windows check covers chain behaviour: a test CA in the current user's Root store, a leaf it signs, and expired, wrong-host and revoked variants.
Why: The read-back catches a silent regression to WebPki. Chain behaviour cannot be a unit test without installing a root.

**Q: Does unifying change Yandex's behaviour, and what is the revocation decision?**
Decision: Yes, in four ways, accepted behind a measurement gate. (a) Revocation: today schannel runs cache-only revocation on every client handshake and ignores unknown status (tls_stream.rs:611-618, 653-681, 716). rustls-platform-verifier checks the end certificate online with an accumulative timeout and a 10 s URL retrieval timeout, and ignores unknown status (windows.rs:161-172, 459-470). A revoked end certificate the responder can answer is refused; an unanswerable one is accepted (soft-fail, as today); a new connection may wait up to about 10 s per end certificate. (b) ClientHello and protocols: TLS 1.2 and 1.3 only; neither stack imitates Chrome's hello. (c) Linux trust uses rustls-native-certs and webpki, not OpenSSL's paths. (d) Error text changes. Unchanged: cookies, User-Agent, pacing, per-phase timeouts, captcha detection and status handling, download caps.
Why: The check gets stronger and matches Windows browsers at a connect-time cost. Fail-closed revocation and disabling revocation were rejected.

**Q: What is the exact change list?**
Decision: (1) The `ureq` entry in `[workspace.dependencies]` becomes ureq with default-features off and features rustls-no-provider, platform-verifier, cookies. (2) Workspace rustls 0.23 with features ring, std, tls12, logging. (3) crates/yandex/src/http.rs: imports, rewritten comment, provider and verifier in agent(), read-back test. (4) deny.toml: CDLA comment updated; bans for native-tls, openssl and openssl-sys with no wrappers. (5) Cargo.lock regenerated by cargo. Removed: native-tls 0.2.18, openssl 0.10.81, openssl-macros, openssl-sys 0.9.117, foreign-types and its shared crate, schannel 0.1.30, der 0.8.3, pem-rfc7468, base64ct 1.8.3; openssl-probe removal depends on rustls-native-certs 0.8. Stays: security-framework, security-framework-sys, core-foundation, core-foundation-sys, webpki-root-certs (kept by rustls-platform-verifier's target-specific dependencies). Added: ring, rustls, rustls-webpki, rustls-platform-verifier 0.7.1 and its Android pieces (jni 0.22 on Android, not optional), rustls-native-certs 0.8. (6) libssl-dev removed from docker/apt-packages.txt:11, tools/profiling/ci_setup.py:36 and docs/reference/PROFILING.md:218 only after the Linux CI passes without it. (7) WEB_CLIENT.md TLS item marked decided and claims corrected. (8) The server crate builds ServerConfig as described below.
Why: Each item follows from the decisions. cargo tree -i is the check, since the lock records target-specific dependencies for every platform.

**Q: Can the axum/hyper server speak TLS with ALPN h2 on Windows, and how does it dispatch?**
Decision: Yes. A tokio TcpListener feeds a connection cap (semaphore), then a tokio-rustls acceptor over a rustls ServerConfig: ring named explicitly, TLS 1.2 and 1.3 only, no client authentication, chain and key read from PEM, alpn_protocols [h2, http/1.1]. The handshake runs under a timeout. Dispatch is on the negotiated ALPN: h2 gets HTTP/2 only; http/1.1 or no ALPN gets HTTP/1.1 only. The LAN port accepts no plaintext. Proposal, not measured: cap 64, handshake timeout 10 s.
Why: ALPN is what the browser declares, so dispatching on it removes hyper's preface-based fallback as a second decision point. Native-tls, axum-server and a reverse proxy were rejected.

**Q: How is the LAN certificate written, loaded and kept safe?**
Decision: Certificate chain and key are two PEM files. Each is written to a temp file in the same directory, flushed (FlushFileBuffers), then renamed over the old file. At start, a certificate that fails to parse or a key that does not match (rustls refuses it in with_single_cert) stops the LAN listener with a notice; loopback and the catalog keep running. The key file is restricted to the owner by an explicit ACL set at creation (mechanism open). rustls keeps no key container, unlike native-tls's per-process container.
Why: The certificate file is the only persistent TLS state, so only its write needs atomicity. Regenerating on each start breaks phone trust; failing the whole server is unnecessary.

**Q: Does the loopback listener between the desktop and the server use TLS?**
Decision: No. It binds 127.0.0.1 and serves plain HTTP with the per-start token on every request. It does not share the LAN certificate. The owner must confirm this.
Why: Loopback does not leave the host, and a same-user process can read the token anyway; Windows lets other local users reach a loopback port, so the token is the control.

**Q: What happens in each TLS failure case?**
Decision: Crash mid-upload: the Yandex client keeps no disk state; the cookie jar is memory-only, so restart starts anonymous. Sleeping phone: idle connections close after a read timeout and unfinished handshakes time out, freeing cap slots. Restarted desktop: the loopback token changes and is handed over; the LAN certificate is unchanged on disk. Hostile LAN peer: plaintext refused; cap and handshake timeout bound half-open handshakes; TLS 1.2 and 1.3 only. A blocked OCSP responder makes an unknown-status chain pass (soft-fail, documented). Full disk: an unwritable certificate stops the LAN listener with a notice; a download that cannot be stored is a download error. A captcha page is read as a captcha before its status.
Why: Each case is handled where its state lives. Timeouts and caps are proposals.

**Q: Keep ureq for V1, or move the Yandex client to one async stack now?**
Decision: Keep ureq for V1; the client stays blocking. The question reopens if the Yandex client moves into the server process, where hyper with a connector on the server's runtime becomes the end state, at a stated rewrite cost (client.rs, fetch.rs, http.rs, about 330 lines with tests; pacing and timeouts rebuilt; cookies via cookie_store).
Why: A rewrite buys one connector and one timeout model, not a better search, and changes the timing the captcha-sensitive search depends on. Shelling out to curl and reqwest were rejected.

**Q: Is WEB_CLIENT.md's claim about native-tls on Windows correct?**
Decision: No. native-tls's Windows backend does have a server accept path with ALPN under the alpn-accept feature (schannel.rs:334-370). The decision against native-tls for the server still holds: the blocking acceptor, the chain FIXME (schannel.rs:360), the RSA-only identity import, and a per-process key container. Doc sections to correct: WEB_CLIENT.md lines 38, 299, 325-326, 383-387 and 572; lines 385-387 call rustls pure Rust, which holds only for the crate and not for ring's C and assembly.
Why: A wrong fact cited as a reason must be corrected even where the choice stands.

**Q: How is the verification sequenced?**
Decision: Ship the Yandex TLS change first as its own commit, then the LAN listener. Static checks on Windows: cargo tree -i for native-tls, openssl, openssl-sys, schannel and webpki-roots (empty), ring (rustls only); cargo build and test for pg-yandex; just lint; cargo deny with new bans; Linux CI without libssl-dev. Manual root checks with the test CA. Live checks for the owner: one search, download and upload on old and new builds, paired, about 50 searches per build (proposal); a network capture for OCSP or CRL traffic; badssl expired and revoked hosts as expectations. Server checks: curl --http2 ALPN, plaintext refused, cap held, truncated certificate file leaves loopback serving.
Why: Each check tests one claim. The captcha risk can only be settled live.

Open:

- Whether every Yandex result host validates under rustls-platform-verifier as under schannel (i.ytimg not reproduced).
- Revocation latency: whether the accumulative timeout bounds the total within the 30 s whole-request bound, and how often Yandex or result certificates trigger online fetches.
- Yandex's anti-bot response to the new ClientHello and revocation traffic (captcha rate; must be measured before shipping).
- Crate closure is from manifests and the lock, not cargo; rustls-native-certs and platform-verifier-android dependencies, including openssl-probe, unverified in full.
- unversioned_rustls_crypto_provider is unversioned in ureq; each rustls bump needs a check.
- ring MSVC build and linkage with the dynamic C runtime on windows-2025 not built.
- ring versus aws-lc-rs long-term maintenance not checked.
- axum not in the local cache; the server integration must be checked when the server crate lands.
- Negotiated-ALPN accessor and server builder names not re-read.
- h2 over rustls on Windows not run.
- Phone trust of the LAN chain: decided in the auth area.
- Loopback plain HTTP: owner confirmation.
- Key file ACL mechanism belongs to the auth design.
- Session tickets for faster resumption: rustls ticketer not verified.
- TLS version policy for Yandex servers (1.2 and 1.3) assumed, not tested; schannel default protocol range not read.
- Timeout and cap values (cap 64, handshake 10 s) are proposals.
- WEB_CLIENT.md line numbers from design reads, to be re-read before editing.

Measure first:

- Paired live comparison of current and rustls builds: about 50 searches per build; handshake time, first byte time, captcha count, status counts, result counts (thresholds: no increase in captcha or error rate; no search past 30 s from revocation).
- Network capture during a search and a download: OCSP or CRL request count and latency.
- Revocation first-connect wait per new end certificate.

Validation notes:

- Corrected by validators: client.rs:22 is the pacing constant; the jar is at client.rs:27-31 and 36-41. Transport text is text.rs:353, not 376 (376 is the SearchError::Request arm). rustls-native-certs-0.8.1 is in the cache (openssl-probe question can be settled from its Cargo.toml); rustls-platform-verifier-android 0.2.0 is absent from the cache.
- WEB_CLIENT.md 325-326 contains no native-tls claim; the list should drop them or name the error.
- Contradiction: LAN certificate paths are placed in gallery.json by auth but moved to server.sqlite by device; the TLS design does not place them.
- Spot-verified in this pass: the `ureq` entry in `[workspace.dependencies]` (native-tls, cookies); schannel.rs:360 FIXME "probably missing the certificate chain" (native-tls 0.2.18); cargo SETTINGS synchronous NORMAL at schema.rs:128 (used by later areas).

---

### Authentication system

**Q: Does the codebase have a multi-user or account concept, and which principals exist?**
Decision: No multi-user or account concept; v1 adds none. A closed enum Principal with Local (loopback router for desktop and CLI, launch token, no device row, fixed snapshot key 'desktop') and Lan (one logged-in browser: device, session, elevated_until_ms). Elevation is a timestamp on the Lan session, not a third principal. Each listener builds its own router; the LAN router never mounts a Local-only route.
Why: No user, account or role table exists (schema.rs:490-697); the data folder is per Windows account. Route absence is stronger than a check. Rejected: a users table, role column, a single router with per-handler checks.

**Q: Where do the owner password hash, epoch, devices, sessions and login failures live?**
Decision: A separate SQLite file auth.sqlite beside gallery.json, with its own accessor in crates/library/src/layout.rs. A new crate pg-auth owns the schema, hashing, tokens, the rate limiter and the store with no HTTP dependency. Pragmas: WAL, synchronous FULL, foreign_keys ON (the catalog keeps NORMAL). Lifecycle is pg-dbfile's judge(), shared lock, exclusive probe for offline writers. Tables: owner (one row), devices, sessions, login_failures.
Why: The catalog is the wrong home (rebuild copies only CARRIED tables and the backups copy the hashes); gallery.json and JsonStore fall back to defaults on an unreadable file and keep failed changes in memory. Cost accepted: deny.toml must list pg-auth beside the rusqlite wrappers. Rejected: Credential Manager or DPAPI for the password, environment variables.

**Q: What happens when auth.sqlite is missing, damaged or foreign?**
Decision: Fail closed for the LAN. The LAN starts only when server.db judges usable and has the owner row. Missing while LAN is enabled: LAN stays down; no empty file is created. Damaged or foreign: moved aside (never overwritten), LAN stays down, notice posted. Loopback always starts so the owner can set a password. Creation only through an explicit owner setup on loopback.
Why: The dangerous state is LAN on with no credential. The start-over policy used for the thumbnail cache would reopen setup to LAN peers.

**Q: How is the owner password first set and reset?**
Decision: Setting or resetting is only possible on loopback. The LAN router has no setup or reset route. The LAN refuses to start without an owner row, and enabling it is refused until one exists. Three entry points share one code path: desktop (loopback), CLI while the server runs (loopback with token), CLI while the server is down (direct write under the exclusive probe).
Why: Setup on the LAN is a first-come race; this matches the decided login (owner sets on desktop, phone only logs in). Rejected: a one-time code shown on the desktop (pairing by another name), recovery codes.

**Q: Which hashing algorithm, parameters and password rules?**
Decision: Argon2id through the argon2 crate 0.5.3 (latest stable in the local index cache). PHC string stored in owner.phc. m=131072 KiB (128 MiB), t=3, p=1, made the default in this pass (the owner may revisit): the cost accepted at O3 in `WEB_CLIENT_TECH_DECISIONS.md`, 0.59 s measured there (argon2-cffi 25.1.0); OWASP floor 19 MiB, t=2, p=1 (reported by a design, not rechecked). Verifications run on a dedicated blocking pool with concurrency 1 and a queue of 4; a full queue gets 429 with Retry-After. Rehash on success if parameters are weaker, guarded by the epoch. Passwords are NFC-normalised on the server. Rules: at least 15 characters after NFC (proposal), at most 1024 bytes, no composition rules; login body capped at 4 KiB.
Why: Memory-hard, bounded concurrency bounds CPU and memory a LAN peer can force. The 15-character floor is the single-factor guard for deletes and moves.

**Q: Session token, digest, CSRF value and launch token: generation and comparison?**
Decision: Session token: 32 bytes from getrandom, base64url; the server stores only its BLAKE3 digest as the primary key of sessions. Minted only at login; never rotated during a session. CSRF value derived, not stored: blake3::derive_key with context 'gallery csrf v1' over the session token (signature to be checked). Launch token: 32 bytes, memory only, compared raw in constant time with constant_time_eq 0.4.2. CSRF value compared in constant time.
Why: Derived CSRF cannot drift from the session; rotation would race the viewer's eight preloads (WEB_CLIENT.md:571). Rejected: raw tokens in the database, HMAC with a server key, per-request rotation, stored CSRF digests.

**Q: Session cookie name, attributes and lifetime?**
Decision: Name `__Host-gallery_session`; Secure, HttpOnly, SameSite=Lax, Path=/, no Domain; Max-Age equal to the absolute lifetime. No other cookies from the gallery except `gallery_device` (see the device cookie decision below).
Why: The `__Host-` prefix forbids Domain and non-root paths, so sibling hosts cannot set it. Lax keeps the cookie on top-level GET navigations, so an external link to a photo arrives signed in; Strict was rejected for that reason (WEB_CLIENT_TECH_DECISIONS.md B3).

**Q: Session lifetime, renewal and time source?**
Decision: Idle 30 days, sliding on use; absolute 90 days from login, not extended (the lifetimes accepted at O2 in `WEB_CLIENT_TECH_DECISIONS.md`). last_seen written at most hourly. Expiry uses the server's wall clock in unix milliseconds passed in as now_ms. Elevation lasts 10 minutes. WebSocket rechecks its session every 60 s and on device revocation. The rest are proposals.
Why: The absolute cap bounds a stolen token; the idle window keeps phones from relogging. Wall clock is needed because expiry must survive restarts. Rejected: sliding without an absolute cap, monotonic time, per-request writes.

**Q: Per-request session check?**
Decision: One indexed lookup per request on a read-only pool, by token digest, joined to devices and the owner row for the epoch. No in-memory session cache; the only in-memory auth state is the set of revocations not yet committed.
Why: A revoke or password change takes effect at the next request with no cache to diverge after a failed commit.

**Q: Device records: shape, identity and one device per browser?**
Decision: devices: id (16 random bytes hex), label (sanitised user-agent summary, at most 200 characters), created, last_seen. sessions: token digest (primary key), device id (ON DELETE CASCADE), owner_epoch, timestamps, elevated_until. A login writes a new device and session in one transaction; a presented cookie's device is deleted in the same transaction, so one browser holds at most one device row. The device id is the snapshot client key (closing the open point in WEB_CLIENT.md:197-198). The desktop has no device row. A device without a live session for 24 hours is pruned.
Why: One row per cookie makes revocation exact. Rejected: a device cookie that survives logins, IP-derived ids, tombstones, the desktop as a device row (it could not be revoked). Note: device design differs (see validation).

**Q: Revocation of one device and the fail-closed rule?**
Decision: Ordered steps: (1) add the device to an in-memory deny set; (2) one transaction deletes the device, cascading to sessions; (3) close its WebSockets with 4401, release its snapshots, drop queued thumbnail work for it; (4) requests from a denied device get 401 whether or not the database still shows the session. The deny entry is removed only after the commit succeeds. If the commit fails: the LAN listener closes every LAN connection and stays down, the device stays denied, a notice shows the reason, and the commit is retried until it succeeds. The LAN reopens only after a successful commit. Sign-out-everywhere is an epoch change.
Why: The current code discards save results (settings.rs:16-18) and JsonStore keeps failed changes in memory (store.rs:51-58); revocation must fail closed. Rejected: commit-first ordering, revocation by a flag checked later, relying on expiry.

**Q: Password change, reset and the owner epoch?**
Decision: The owner row holds phc, epoch and changed_ms. A change from a Lan session needs the current password, verified through the queue; then one transaction runs UPDATE owner SET phc, epoch = epoch+1 WHERE epoch = `<the epoch read>`. Zero rows changed is 409 and the caller retries. The same transaction deletes every device except the caller's and updates the caller's session epoch. Reset from loopback needs no current password and deletes every device, clearing login_failures. Every change posts a notice.
Why: Compare-and-set makes concurrent changes retry instead of overwriting; one transaction is all-or-nothing. Keeping the caller's device follows the current password as proof. Rejected: a phone reset without the current password, last-writer-wins on the hash.

**Q: Resetting the password when the server is down, and CLI writes?**
Decision: 'gallery auth set-password' reads the new password with echo off from a console only (refuses when stdin is not a console), never from argv or environment; enforces the 15-character minimum. It probes with pg-dbfile's non-waiting Live::exclusive on server.db. If the probe fails, a server holds the file and the CLI sends the change to loopback with the launch token (re-reading the launch file and retrying once on 401). If the probe succeeds, the CLI writes directly in the same single transaction. The CLI refuses offline writes under WSL on a Windows-mounted home, since locks are per operating system.
Why: The probe is the non-waiting primitive; Lock::take waits about five seconds (lock.rs:11-13, 27-38) and is wrong for a probe. Rejected: a separate lock protocol, refusing CLI auth commands while the server runs.

**Q: CSRF protection for cookie-authenticated requests?**
Decision: Layered. (1) SameSite=Lax on the cookie (WEB_CLIENT_TECH_DECISIONS.md B3). (2) Non-GET requests and the WebSocket upgrade must carry Origin equal to the configured origin; missing Origin refused. (3) Sec-Fetch-Site, if present, must be same-origin. (4) Non-GET bodies must be application/json. (5) Non-GET requests carry the X-Pg-Client header (WEB_CLIENT_TECH_DECISIONS.md B4); there is no derived token. (6) The WebSocket is read-only for catalog state; mutations go over HTTP.
Why: Each layer covers a different bypass: old browsers without Sec-Fetch-Site, form posts, sibling origins, non-browser forgery. Rejected: double-submit cookie (needs a readable cookie), SameSite alone, per-form tokens, mutations over the socket.

**Q: Rate limiting and lockout?**
Decision: No account lockout. Per source address, failures counted in login_failures (persisted across restarts). After 5 consecutive failures the address waits 30 s, doubling to a 15-minute cap (proposal); success clears the row; counts reset after 24 hours with no failure. Globally, verifications are serialised and a failed verification delays the next by at least 1 s; a correct password is never refused by the global gate. A full queue gets 429. A sustained run posts one notice.
Why: Persisted counters survive restarts; per-address backoff stops one device; delay rather than refusal keeps the owner able to log in. Rejected: account lockout, CAPTCHA, trusting X-Forwarded-For, in-memory-only counters.

**Q: What does the LAN listener serve before login, and the Host and TLS rules?**
Decision: Without a session only: GET /login (a separate page, not the app bundle) and POST /api/auth/login, and nothing else (`WEB_CLIENT_TECH_DECISIONS.md` B9; `WEB_CLIENT_RECONCILED.md` 1.2). GET /api/auth/session needs a session. Every other path, including the bundle, workers, thumbnails, listings and the WebSocket upgrade, answers 401 with one JSON body, and unknown paths get the same 401. TLS only, no HTTP listener and no redirect. Host must equal the configured LAN name or address, else 421 with an empty body. No CORS header is ever sent. Pre-login connection, handshake and header limits apply.
Why: The app bundle would expose product code before login; the Host check defeats DNS rebinding; one 401 body gives no route map.

**Q: The loopback listener, launch token, launch file and single-instance rule?**
Decision: Loopback binds 127.0.0.1 on a random free port chosen per start, with its own router. A 32-byte launch token, in memory, carried only as an Authorization: Bearer header on every loopback route, GET included. The loopback router rejects requests with an Origin header, requires Host 127.0.0.1:port or localhost:port, and never reads cookies. The launch file `<home>`/launch.json holds the port and token, written temp-then-rename, owner-only DACL on Windows (new code, created before bytes are written) and mode 0600 on Unix, deleted on clean stop. The LAN router never reads it. Single instance: pg-dbfile's Lock on the home folder before the LAN binds, refusing with a message if another server runs.
Why: Random port stops a squatter receiving the token; Host blocks DNS rebinding; bearer headers cannot be sent cross-origin without preflight. Rejected: fixed port, token in a URL or cookie, one credential for both listeners.

**Q: The LAN settings and where the LAN flag lives?**
Decision: LAN enabled flag (default off), bind address and port, the LAN name for the Host allowlist, and certificate paths are server-owned settings in gallery.json, written only by the server. The auth state is never in gallery.json.
Why: Default off makes JsonStore's default fallback safe for this file; only the credential file must fail closed. Note: device design moves gallery.json to server.sqlite (see validation).

**Q: TLS requirement for login?**
Decision: The LAN serves TLS only and does not start without a certificate it can serve. Login is refused on non-TLS connections, so Secure is always satisfiable. Cookies bind to the host, not the certificate, so renewal does not log the phone out; an address change requires a new login.
Why: Login over plain HTTP sends the password in clear. The repo has no certificate or TLS server code, so the claim at WEB_CLIENT.md:274 that a private CA is configured is unsupported and this design does not rely on it.

**Q: Which actions need the owner, and at what level?**
Decision: Loopback only: enable and disable the LAN; bind address and port; certificate paths; set or reset the password without the current password; sign out everywhere without a Lan session; server stop; restore of server.db. Ordinary Lan session: reads, ratings, marks, rotation, scans and imports, Yandex, library folder add and remove, deletes to the Recycle Bin, device list read, rename of own device, logs. Elevated Lan session (password re-entered within 10 minutes): write_back, revoking another device, sign out everywhere. Security parameters are constants, not settings.
Why: The split follows blast radius; a delete goes to the Recycle Bin, so it stays restorable in Explorer. Rejected: elevation for every delete, LAN enable from a phone, settable security parameters.

**Q: Logout?**
Decision: POST /api/auth/logout with Origin and CSRF checks. Deletes the caller's device (cascading to its session), returns 204 with the cookie cleared (same attributes, Max-Age=0), and sends Clear-Site-Data with cache, cookies and storage. The page also deletes its own Cache Storage and IndexedDB entries.
Why: POST so a cross-site page cannot log the owner out. Clear-Site-Data behaviour on Android Chrome is unverified, so the page-side deletion is the reliable path.

**Q: Login endpoint and responses?**
Decision: POST /api/auth/login with {password}. Checks in order: Origin, JSON content type, 4 KiB body cap, rate limits (before verification), verification through the queue. On success one transaction writes the device and session (deleting the presented cookie's device) before the response: 200 with Set-Cookie and a JSON body with the CSRF value. Every failure is a single generic 401. Throttled: 429 with Retry-After. Store unwritable: 503, no cookie. Every auth response is Cache-Control no-store. No label field; the label comes from the user agent.
Why: Writing before responding means a cookie is issued only for a committed session. No oracle from failure bodies.

**Q: Step-up (elevation)?**
Decision: POST /api/auth/elevate with {password}, verified through the queue; sets the session's elevated_until_ms to now plus 10 minutes. Elevated routes check that timestamp in the same lookup. Wall-clock based, so sleep ends elevation. No token rotation, no new cookie.
Why: Rotation on elevation breaks in-flight requests. Per-action prompts are poor on a phone.

**Q: Cache and response headers?**
Decision: Auth responses no-store. Listings, JSON data and view endpoints private, no-store. Thumbnails private, max-age=604800, immutable (O11 option (a); changes WEB_CLIENT.md from public). Every LAN response: Content-Security-Policy default-src 'self', frame-ancestors 'none', base-uri 'none'; X-Content-Type-Options nosniff; Referrer-Policy no-referrer. No HSTS.
Why: public would let a shared cache store authenticated responses. HSTS does nothing for an IP-literal host (not checked here).

**Q: Owner notices and logging?**
Decision: Notices go to the desktop inbox (crates/app/src/inbox.rs:285, 341) with new Notice variants for: LAN not started and why; server.db moved aside; password set, reset or changed; device signed in; device revoked; sign-out-everywhere; revocation not saved with LAN closed; throttling engaged. Auth events go to gallery.log with device id, route, outcome and source address for failures. Logs never contain passwords, cookies, CSRF values, launch token or derived values; one redaction function strips Cookie, Authorization and X-Pg-Client headers and /api/auth bodies (no redaction exists today).
Why: The inbox is the existing owner channel; gallery.log is shared on fault reports.

**Q: Crash and disk-full behaviour?**
Decision: Each auth write is one transaction. Before commit the old state holds; after commit the new holds and an orphan device from a lost response is pruned. Fault-injection test at each commit boundary in the writeback crate's style. Full disk: login returns 503 with no cookie; revoke follows fail-closed; last_seen is best effort. A failed launch-file write means loopback does not start and the desktop reports it.
Why: Login refuses rather than issuing a session that would vanish on restart and could not be revoked.

**Q: Cookies in relation to the desktop and CLI, and the code's home?**
Decision: New crate crates/auth (pg-auth) holds the credential logic, hashing, tokens, digest lookup, rate limiter, store, pg-dbfile holds and the offline writer; no HTTP dependency. The server crate holds routers, cookie and header rules, WebSocket and the loopback listener. pg-cli depends on pg-auth for the offline command, added in run_command.
Why: Credential logic is testable without HTTP and the CLI reuses the same transaction rule.

**Q: Test strategy?**
Decision: Unit tests: cookie attribute string; Origin and Sec-Fetch-Site rules; Host allowlist; limiter under an injected clock; digest lookup; epoch compare-and-set; PHC rehash; NFC. Fault tests at each commit boundary in login, revoke, password change and reset. Integration: desktop reset; both CLI reset paths; revocation with a failing commit; full disk via a low max_page_count (proposal, not run).
Why: Crash, disk and revocation rules are the risky parts and need deterministic tests.

Open:

- Android Chrome accepts the Secure, `__Host-`, SameSite=Lax cookie on https://`<LAN-IP>` with the private CA (P7 phone trust gate; unverified).
- Whether Chrome for Android sends Origin on every non-GET and on the WebSocket upgrade, and Sec-Fetch-Site (refusal of missing Origin is the safe default).
- Clear-Site-Data behaviour on Android Chrome for Cache Storage and HTTP cache.
- Owner decisions: step-up tier (chosen here); deletes and moves at ordinary level (chosen); no recovery code (rejected by default); last IP not stored in v1.
- Argon2 cost: 128 MiB, t=3, p=1, the default made in this pass (the owner may revisit); 0.59 s measured at B8 in `WEB_CLIENT_TECH_DECISIONS.md`, on one machine only; OWASP floor not rechecked.
- All other numbers are proposals (30 days, 90 days, hourly last_seen, 10 minutes, 15 and 1024, queue of 4, backoff, 1 s gate, 24-hour prune, 60 s WebSocket recheck, connection limits).
- argon2 0.5.3 dependency closure read from the local cache only; cargo deny not run. unicode-normalization is not locked and unchecked.
- getrandom 0.4.3 and constant_time_eq 0.4.2 direct-use signatures and blake3::derive_key signature unverified.
- Windows owner-only DACL: Win32_Security_Authorization is not enabled in windows-sys features; calls not written; PG_HOME behaviour untested.
- No-echo console read: SetConsoleMode calls not written; rpassword is an unchecked alternative.
- Restoring an older auth.sqlite revives revoked devices and an older password; not detected in v1.
- A restart while a revocation is uncommitted reinstates the device (the LAN is closed in that state).
- Same-user malware can read the launch file and reset the password through loopback (not ruled out).
- A stolen unlocked phone works until revoked or the absolute cap; only elevated actions are protected.
- Offline guessing if auth.sqlite leaks; only Argon2id cost and the 15-character minimum defend it.
- LAN attacker can delay login through the global gate and queue (429 during floods).
- Clock set backwards extends sessions.
- Cookies not isolated by port; the design assumes a dedicated host name.
- Two Windows accounts each run a server; the second cannot bind the LAN port.
- Whether the server runs after the desktop closes (WEB_CLIENT.md:268 says always-running; no server code exists yet).
- Single-instance rule uses Lock::take (five-second wait; a new use of an existing primitive).
- Offline reset across operating systems (WSL cannot see a Windows lock); refusal rule untested.
- Whether any Web Worker issues a non-GET request, which would need the CSRF header passed in.
- CSP needs of the bundle (wasm-unsafe-eval) and whether the login page can use 'self' only.
- Passwords and tokens in memory are not zeroed; zeroize is an unchecked dependency.
- Password manager autofill attributes assumed (current-password, username); untested.
- Panic log capture in the desktop (alert.rs, lib.rs) not read; logging rule cannot be verified until a server trace layer exists.
- New Notice variants must follow the existing enum shape (notice.rs not read in full).
- WAL with synchronous NORMAL can lose committed transactions on power loss (reason for FULL on auth.sqlite; not rechecked).
- OWASP, NIST and RFC 6797 statements were recalled, not checked; the 15-character floor rests on the single-factor argument.
- `__Host-` and SameSite behaviour in current Chrome for Android is expected, not verified on a device.
- Device label parsing rules not designed.
- The public login page reveals that the product runs (accepted).

Measure first:

- Argon2id verification time and peak memory on the owner's desktop at 128 MiB, t=3, p=1 and at the OWASP floor, one at a time.
- Queue wait for a correct password behind a 20-attempt flood.
- Per-request session lookup latency on the read pool, LAN and loopback.
- fsync cost of synchronous FULL for a login and a revoke on the target disk.
- Time from revoke response to the first 401 and the WebSocket close.
- last_seen write rate at the hourly setting.
- Phone trust gate on Android Chrome: cookie acceptance, Origin and Sec-Fetch-Site presence, Clear-Site-Data effects.

Validation notes:

- Corrections: the fault module is declared at writeback lib.rs:56-57 (53-54 are use statements). run_command is at cli lib.rs:131; the Scan, Remove, Import and Thumbs arms are at 142, 208, 226 and 242. Layout struct fields are 19-22.
- Contradictions: (1) Decided item: the exclusive hold is only for layout changes, but the auth offline reset takes an exclusive probe on server.db; restate the rule or change the design. (2) write_back is settable from any client, but here it needs elevation; the owner decides. (3) Device design: desktop is a device row (random key, server.sqlite) versus no device row here (fixed 'desktop' key). (4) Cookies: auth sets one `__Host-gallery_session` (Strict); device sets `__Host-pg_sign` (Lax) plus pg_device (Strict, Path=/api/login). (5) Sign-in lifetime: resolved at O2, 30 idle and 90 absolute for both areas (`WEB_CLIENT_RECONCILED.md` O2; the device area's 180 is void). (6) Login with a presented cookie: auth deletes and recreates the device; device reuses it. (7) Logout: auth deletes the device; device keeps it. (8) Password change: auth deletes other devices; device deletes sign-ins only. (9) Loopback reset: auth needs no current password; device requires it. (10) Revoking another device: auth requires elevation; device does not. (11) Pruning: auth prunes after 24 hours; device forbids it. (12) Rate-limit state: auth persists; device keeps it in memory. (13) Failed revocation commit: auth closes the LAN; device keeps LAN up and returns 503. (14) CSRF: auth derived header; device custom header with no derived value. (15) Route names: `/api/auth/*` versus /api/login, /api/me, /api/logout. (16) Single-instance lock: Lock::take versus a new non-waiting lock. (17) Snapshot key: 'desktop' here is void if the snapshot design is accepted. (18) SameSite of the session cookie: Strict here; realsize's argument assumes Lax. (19) Thumbnail Cache-Control: auth private, realsize's views private with different max-age. (20) TLS certificate paths: gallery.json here, server.sqlite in device.
- Corrections in view of the summary: "public" for thumbnails is replaced by private in this design.

---

### Device identity

**Q: What is the device record's key, where is it generated and stored?**
Decision: The device record's key is a 32-byte random secret, generated server-side from getrandom (already locked as the `getrandom` package in Cargo.lock; becomes a direct dependency). The secret is the device cookie's value (see the device cookie below) and is never stored. The devices table holds its BLAKE3 digest as the device id, so a copy of the table cannot authorise a request. Generated inside the insert transaction; a collision is an error, never a reuse. Clients never supply the key or the id; the id identifies and never authorises.
Why: One CSPRNG path for ids and credentials; catalog_id is not a device key because a readable rebuild keeps it. Rejected: client-chosen ids, sequential ids, hashes of user agent or IP.

**Q: Where does server state live, and what goes in it?**
Decision: One SQLite file, `<home>`/server.db, beside gallery.json and outside the catalog folder. Holds: devices (BLAKE3 digest of the key, kind this_computer or browser, name, created, last_seen, revoked_at, a typed JSON prefs column); sign_ins (one row per browser device; BLAKE3 hash of the sign-in secret; idle and absolute expiry; UNIQUE(device_id)); meta (owner_set marker, owner password verifier, account epoch, cutover import marker); settings (one JSON row for owner-wide and server-wide preferences). gallery.json is read once at cutover and kept unchanged. Saved views and the session listing stay in the catalog, where rebuild carries them (schema.rs:680, 688; rebuild.rs:146-149).
Why: Device, sign-in, preferences and owner credential need one transaction domain. The catalog is the wrong place (rebuild replaces the file; only CARRIED tables survive). JsonStore is wrong because quarantine would reset auth and every device at once. write_back is moved here because its default is true (prefs.rs:101).
Rejected: devices in the catalog, gallery.json, one file per device, preferences split across files.

**Q: What is a device, what kinds exist, and when is each created?**
Decision: A device is one client installation with its own preferences and snapshot budget. A browser device is one browser profile on one phone (tabs of a profile share it). The desktop is one device of kind this_computer, created at first server start with the same key across restarts. A browser device is created in the login transaction after the password verifies. Login reuses an existing browser device only when the presented device cookie names one that is not revoked.
Why: The device must survive sign-in expiry or every expiry resets preferences and the budget. Reuse is limited to non-revoked devices, so a revoked phone cannot return through its old cookie.
Rejected: device equals sign-in; device created on any login request; several sign-ins per device.

**Q: How do the cookies tie to the device, with attributes and lifetimes?**
Decision: Two cookies. Sign-in cookie `__Host-gallery_session`: 256-bit random from getrandom; server stores only its BLAKE3 hash; HttpOnly, Secure, SameSite=Lax, Path=/, no Domain; idle 30 days sliding (last used written at most hourly), absolute 90 days (O2, `WEB_CLIENT_TECH_DECISIONS.md`); Max-Age equals the absolute value. Device cookie `gallery_device`: the raw device key. The key is a secret: the cookie carries it, and the server stores only its BLAKE3 digest in the devices table, so a copy of server.db cannot be replayed as a cookie. HttpOnly, Secure, SameSite=Lax, Path=/api/auth/login (no `__Host-` prefix, since the path is not /), Max-Age 400 days refreshed at each login. Only POST /api/auth/login reads the device cookie; it grants nothing (names per WEB_CLIENT_TECH_DECISIONS.md B3). Logout deletes the caller's sign-in and keeps the device and its cookie.
Why: Identity is long-lived; the credential is short-lived; the device cookie travels only on the login path. Rejected: one cookie for both jobs, a hashed device cookie, a rotating refresh chain (can be added later), localStorage device ids.

**Q: How does the desktop authenticate over loopback, and where is the launch token?**
Decision: The desktop has no cookie. Its credential is a 256-bit launch token from getrandom, minted per server start. The server keeps only H = blake3(token), in memory. Token and port go in `<home>`/launch.json, written through TempFile and persist (platform/src/file.rs:84-109), with an owner-only DACL on Windows set at creation and mode 0600 on Unix. Attach handshake: the desktop sends a fresh 32-byte nonce; the server answers blake3::keyed_hash(H, nonce); the desktop computes H from its token and checks the answer before sending the bearer. After a 401 the desktop re-reads launch.json. launch.json sits in the home, so each home has one server and one token file.
Why: A squatter on a stale port never sees H or the token. The handshake key is H because the server holds only H (the earlier input's keyed_hash(token, nonce) would require storing the token, contradicting hash-only). An explicit DACL is needed because PG_HOME can point anywhere. Rejected: desktop logs in with the password, a cookie for loopback, a named pipe or Unix socket, the launch file pinned to the default home.

**Q: Which preferences are per device, owner-wide, and where does real_size go?**
Decision: Per device (device prefs column, serde defaults): row_height, dev_mode, real_size (proposal: real_size per device, owner to confirm). Owner-wide (settings row): write_back (server-wide, owner credential), show_borders, last_file, yandex_folder. The yandex_folder change from WEB_CLIENT P1 is deliberate: start_folder reads and checks it on the server host (yandex.rs:520-527) and save writes into it on the server (yandex.rs:533-550). The import at cutover seeds this_computer's row and the settings row once, in one transaction, with a marker in meta. The serde modules and validation in prefs.rs:46-96 are kept and moved.
Why: Display-dependent values are per device; a server path is per owner; the owner's intent is server-wide. Rejected: every preference per owner, everything per device, yandex_folder per device, a device-keyed map in gallery.json.

**Q: Which key does the snapshot cap count against, and what releases a snapshot?**
Decision: The device key, derived server-side from the sign-in or launch token. At most four live listings per device across tabs (proposal) and a global entry budget (proposal). A browser device's listings enter a grace period when its last WebSocket closes (length open); dead sockets are found by ping. Revoke or password change releases affected listings at once. The desktop releases its listings when its last connection closes, with no grace. Every use of a listing id checks the owner; a mismatch returns the same error as an unknown id.
Why: A sleeping phone reconnects with the same key on each wake, so per-device counting stops reconnect loops minting budget. Rejected: per connection, per tab, per sign-in, per IP, LRU eviction within a device (silent data loss).
Note: this area depends on server-held listings that the snapshot design removes (see validation).

**Q: Does the event stream need a device key?**
Decision: No. The event sequence is the catalog's change sequence meta.seq, which is catalog-wide. A client records the last number applied and resume(seq) replays from there. Events carry the actor's device key (or none for scan work) and a via label (desktop, cli or browser) for display only.
Why: A per-device sequence would be a second numbering that no source can produce. meta is a carried table, so seq survives a readable rebuild.

**Q: How are devices named, and how does the owner see and revoke them?**
Decision: Names are at most 64 characters after trimming and removing control, format (Cf), bidi-control and newline characters; empty takes the default. A browser device's default is an allowlisted user-agent label (for example "Chrome on Android"); the raw user agent is not stored. The desktop's default is the machine name, obtained by a call not yet in the repo. No IP is stored. Endpoints: GET /api/me; GET /api/devices (key, kind, name, status live, expired or revoked, created, last-seen, attached); PATCH /api/devices/{key} renames; POST /api/logout ends the caller's own sign-in; POST /api/devices/{key}/revoke ends a browser device (any signed-in browser or the desktop may do it); POST /api/devices/revoke-others; DELETE /api/devices/{key} forgets a revoked or expired browser device with its preferences (never a live one, never this_computer). The desktop cannot be revoked or forgotten remotely (403 not-revocable). No automatic pruning.
Why: Names are not keys; allowlisted parsing avoids storing fingerprints; logout and revoke differ; automatic pruning silently deletes preferences.

**Q: What does revoking a device do, in what order, and what happens when the write fails?**
Decision: For one browser device: (1) Fence: under the registry lock, mark the device revoked in memory and remove its sign-ins from the live registry. (2) Drain: take the device's write gate exclusively; every mutation holds the gate shared across its check and commit. (3) Commit one transaction: set revoked_at, delete the sign-in row. (4) On success: close its WebSockets with 4401, cancel its open streams (each checks a cancel token before each chunk), release its listings, release the gate, respond 200. (5) On failure at step 3: keep the fence, release the gate, respond 503 with the reason that the revocation holds only until the server stops; post a notice; retry the commit until it succeeds. Queued thumbnail work is not cancelled; only delivery to the device stops. Scans started by the device keep running as server work.
Why: The fence precedes the drain, so no new write starts after it; failing closed means a failed commit does not reopen a stolen phone. A crash before step 3 leaves the device live after restart; the owner got no 200 and retries. Rejected: delete sign-ins only, commit before fence, rolling back committed writes, cancelling thumbnail work.

**Q: How does a password change interact with sign-ins and a login in progress?**
Decision: Needs the current password, from any signed-in browser or loopback. Steps: (1) under the registry lock, fence every browser sign-in and bump the in-memory account epoch; (2) drain all browser device gates; (3) one transaction stores the new verifier, sets the epoch in meta, deletes every browser sign-in row; (4) close every browser WebSocket with 4401, release browser listings, respond. Devices are not revoked. Login verifies the password outside any transaction, recording the epoch it verified; then a transaction refuses if the epoch changed; the registry adds the sign-in only after commit, under the registry lock, and only if the in-memory epoch still matches; otherwise it deletes its own row and refuses.
Why: The registry lock serialises additions against the fence, so a login that verified the old password cannot leave a live sign-in in either order. Keeping devices means the stolen-phone answer costs one password change, not every phone's identity. Rejected: keeping sign-ins across a change, revoking every device, verifying inside the write transaction.

**Q: Which listener accepts which credential, and what checks apply?**
Decision: The LAN accepts sign-in cookies only. The device cookie is read only by POST /api/auth/login. It rejects bearer tokens before any lookup. Host must equal the configured LAN name or address. Every non-GET request, login included, needs an Origin exactly matching the LAN origin and a custom header such as X-Pg-Client; no CORS preflight is ever allowed. GETs change no state. The WebSocket upgrade checks Origin. The loopback accepts the launch bearer only; rejects cookies; requires Host 127.0.0.1 or localhost with the bound port; rejects any request with an Origin header. Before password verification the LAN creates nothing; setup happens on loopback only.
Why: SameSite does not separate ports on one host; Origin plus header closes cross-port writes. A leaked cookie cannot drive loopback; a leaked token cannot be used from the LAN. Assumption: the desktop's own client sends no Origin (not verified).

**Q: How is one server per home enforced?**
Decision: The server holds `<catalog_dir>`/server.lock for its whole run as an OS exclusive file lock taken without waiting (new non-waiting helper in pg_dbfile). A second server for the same home exits at once with the reason. The desktop attaches through launch.json and the handshake, and starts the server only when attach fails and it takes the lock.
Why: An OS lock is released at process death, so a crash leaves no stale lock. Reusing Lock::take (a five-second wait, a layout-only semantic) and a pid file (pid reuse) were rejected.

**Q: What happens on corruption, missing store, disk full, crash, restart or restored backup?**
Decision: Missing store: first-run state is the owner_set marker in meta, not file absence; the LAN stays off until the owner sets a password on loopback. Corrupt store: the LAN stays off, loopback runs, a notice says why; recovery is an explicit desktop action (move aside with pg_dbfile move_aside, create a new store, set first run). Disk full: login fails closed with 503 and no cookie unless its sign-in committed; revoke answers 503 per the fence rule; preference changes are held in memory with a notice, as PrefStore does; last_used writes are best effort. Crash: a crash before commit leaves the old state and the client got no success; after commit the new state holds and the fence is lost with the process, which is safe because the store is authoritative at start. Server restart: sign-ins and devices survive, the launch token rotates, listings drop and phones reopen on SnapshotExpired. At start: expired sign-ins are deleted and a leftover launch.json temp is removed. A restored backup brings back revoked devices and the old password; not detected.
Why: A missing file must not count as first run to a LAN peer; a corrupt one must not silently drop devices. Rejected: silent recreation, cookies that are not stored, memory-only sign-ins.

**Q: What is the in-memory registry's rule against the durable store?**
Decision: The registry (sign-in hash to device and expiry, device status, epoch) is what requests check; the store is what restarts load. The registry may be ahead of the store for removals and never ahead for additions. Lock order is fixed: registry lock, then device gate, never the reverse.
Why: Revoke and password change fail closed while the process runs; an addition cannot outlive a fence that ran during its commit.

**Q: What must never be logged?**
Decision: Logs record device keys, names and events. They never record a sign-in secret, the device cookie value, the launch token, a hash of any of these, the owner password or a verifier. The request trace excludes Cookie, Set-Cookie and Authorization headers.
Why: Logs are the commonest bearer-token leak path. Device keys grant nothing.

**Q: Is the client cache keyed by catalog_id?**
Decision: Keep it. The client keys its thumbnail cache by catalog_id with the URL and clears when the handshake reports a different catalog_id. The justification is corrected: a readable rebuild keeps catalog_id and file ids (rebuild.rs:204-208), so catalog_id changes only when a rebuild carries no rows, and then the ids are new too.
Why: Cache Storage outlives a rebuild and the thumbnail URL has no catalog id.

**Q: Is real_size per device, and what is decided about the 1:1 button?**
Decision: real_size is per device (a proposal the owner must confirm). The scope is decided; whether a 1:1 button is shown is not decided here (realsize decides it).
Why: One-to-one depends on device pixel ratio.

Open:

- Numbers, all proposals except the sign-in lifetimes (idle 30 days, absolute 90 days, O2): last-used hourly; device cookie 400 days; 32 live browser devices; four listings per device; global budget; grace length (open); name length 64; ping interval; registry-lock hold time.
- Owner confirmation of real_size per device and of yandex_folder moving to owner-wide.
- Whether a 1:1 button is shown (realsize's question).
- Whether revoking a device should need the owner password (step-up); left open.
- Hostname source for this_computer's default name: no hostname call exists in crates/ (grep); Windows API and feature flags unverified.
- Windows owner-only DACL for launch.json: code is new; ConvertStringSecurityDescriptor needs Win32_Security_Authorization, which the `windows-sys` entry in `[workspace.dependencies]` does not list.
- Same-user processes can read launch.json, server.sqlite and desktop memory.
- Whether a home on a synced or shared folder should be refused or warned about.
- Single-instance helper is new code; cross-process behaviour unverified.
- Registry lock contention under load unmeasured.
- Whether hyper and axum send HTTP/2 GOAWAY and cancel in-flight streams per connection (no server source read).
- SQLite rollback and integrity after SQLITE_FULL; WAL on removable or network homes; assumed.
- Chrome on Android accepts Secure cookies and `__Host-` under the private CA (P7); 400-day cookie cap is general knowledge; SameSite=Lax on a Path-scoped cookie unverified.
- Whether the frontend holds more than one listing per device (two tabs share the cap of four).
- Whether a second browser on one phone is a separate device (by browser design, not checked).
- Event log storage and device attribution: no event log code found.
- Password reset, hashing and rate limits (P14) feed device creation; the login-rate state is in memory here.
- Restored backup and downgraded desktop reading stale gallery.json after cutover; no downgrade guard designed.
- Whether the via label should be visible in event rows (owner).
- Migration of gallery.json at cutover: order against WEB_CLIENT P5 is open.
- WEB_CLIENT.md must be updated at lines 194-198, 221-223, 244, 278-281 and 574 (this design does not edit it).

Measure first:

- Listing memory per entry (56 bytes, `crates/model/src/grid.rs:41`).
- Time from revoke request to WebSocket close and to the last write drained.
- Registry-lock hold time under concurrent requests.
- Password-hash time on desktop and phone.
- Commit latency of the hourly last-used write on target disks.
- Windows DACL result for launch.json against an inherited parent ACL and a PG_HOME on another folder.
- That two processes cannot both take the single-instance lock.

Validation notes:

- Corrections: crates/platform/src/testing/lock.rs:2 is wrong as cited; production file locks are in crates/dbfile/src/lock.rs:27-38 and 55-77. The correct statement is that no production server or single-instance lock exists. rebuild.rs views and session are at 148-149 (146 is roots, 147 is folder_failures).
- Verified as cited: prefs.rs 22-40, 98-110, 125-136, 145-157, 164-173; store.rs 29-37, 51-58; layout.rs 38, 52-56, 92-95; dirs.rs 31-32; settings.rs 4 (stale comment) and 16-41; controller.rs 110-116; shell.rs 87-106; yandex.rs 520-527, 533-550; rebuild.rs 87-91, 134-153, 160-161, 204-208; schema.rs 680, 688, 692-694, 701-702; the `getrandom` package in Cargo.lock; WEB_CLIENT.md 194-198, 221-223, 244-246, 271-276, 278-284, 561, 574, 581.
- Contradictions: (1) The device design's open list asks whether desktop writes move in the same step as reads; the decided item says they do, so close the point. (2) Auth design: a full list of disagreements is recorded in the auth validation notes; both cannot be accepted as written. (3) Snapshot design makes the device snapshot-cap decisions void. (4) realsize: real_size per device for all device kinds versus no persisted real_size on phones in v1; gallery.json kept unchanged versus moved into server.sqlite. (5) realsize: the desktop real_size key must match the device design's random desktop id (now settled as random). (6) The device design's stale catalog_id comment in schema.rs:692-694 conflicts with rebuild.rs:204-208.

---

### Listing snapshots: purpose, need and lifetime

**Q: What is a listing snapshot for, and why is one needed?**
Decision: A listing is the ordered membership of one grid: the files in one folder scope (with recursive flag), the Query and the Sort, in final sort order, each with its shown width and height. The grid works in positions (rows, scrollbar, selection, keyboard steps, range fetches), so the sequence must not change under the client while it uses it. It is needed for four reasons: (a) geometry: the greedy layout (client layout.rs:38-53) places every row from all earlier aspects, so no row or scrollbar height can be set until every aspect is known, in order (WEB_CLIENT.md:54-56); (b) index identity: position i names one file only while the order is fixed (a rating edit under a rating sort would otherwise shift every index); (c) consistency: pages read at different times can duplicate or skip; (d) rank: finding a file's position (grid.rs:168) and the selection rule (grid.rs:384-399) need the whole sequence. What is frozen: membership, order and shapes as of one sequence number. What is not frozen: records (rating, marks, path), read by FileId when shown.
Why: The four reasons are properties of the sequence, whichever side holds it. Records stay out because the catalog already holds them and they change. The desktop gets the freeze free from its own Vec; a web client needs the frozen sequence held somewhere it can name. Rejected: holding full records with the list (a second copy of the catalog), treating the list as a rebuildable cache.

**Q: Where does the frozen sequence live: a server-held snapshot or a client-held replica?**
Decision: The client holds the frozen sequence as a replica. The server holds no per-listing state: no listing id, snapshot table, grace period, idle timer, cap or budget. The server answers stateless calls keyed by the listing value (Listing, view.rs:11-19, which is Serialize) plus a sequence number: Full (the sequence at S as FileIds and shapes in final order, with count and digests, in one read transaction); Delta(since S); record fetch by FileId. The replica lives in the layout worker, in memory only. Server-held snapshot with its full lifecycle is the fallback only if the owner rejects this.
Why: Server-held state is what makes the lifecycle hard; every number in designs 1 and 2 is unmeasured. The catalog's change number (meta.seq; files.changed at schema.rs:523-524) already records what changed, so a delta is a query, not a server object. The replica is self-checking through digests.
Rejected: a server-held snapshot (fallback), keyset paging (decision 3), a fetch-whole-each-open client array with no delta (kept as the Full reply, not the only reply).

**Q: Keyset paging on (sort key, order key, FileId) with a count: is it better?**
Decision: No, for listings. Strengths are real (no server state, restart-safe). Decisive weaknesses: (1) geometry needs every aspect in order, so keyset reads all N rows in pages; name order is an index seek (files_order, schema.rs:535) but the other 13 keys are expressions over joined tables with no index (query lib.rs:355-390; indexes at schema.rs:535-543), so each page is a scan and sort; (2) a count gives items, not rows; (3) no random access for a scrollbar jump; (4) under live edits the keys change between pages; (5) no frozen set for bulk actions.
Why: These failures are structural to the sort keys, not to the implementation. The delta protocol gives the same no-server-state property and fixes all five.

**Q: What is the wire identity of a listing, and what does each reply carry?**
Decision: Every listing message carries the listing key (Listing value), the sequence number S the reply is valid at, and for Full and Delta replies the count N and set digest. The client tags its arrays by (listing key, S); a reply whose S differs from the replica's is stale and dropped. The controller's generation (controller.rs:204-205, 334-337) stays client-only and is not sent. The decided (listing_id, generation) stamp (WEB_CLIENT.md:205-206) is replaced by (listing key, S); the freeze of membership and order is kept.
Why: The index belongs to a replica tagged by the sequence it was computed at; a server-issued id is not needed, so no restart reuse and no server state.
Rejected: server-issued random listing ids (server state), counter ids (reused after restart), in-place rebuilds with a bumped generation.

**Q: How is a delta computed and applied so that the replica equals the server's order?**
Decision: A Delta request (listing key, since S) runs in one read transaction, reading S' = meta.seq and the layout epoch. (a) Changed files are rows with changed > S, via the files_changed index (schema.rs:543); each is tested for membership now (scope at browse/read/scope.rs:11-20 and the query condition): a match is an insert or move with its final index equal to its rank in the full order at S'; a non-match has left. (b) Tombstones with seq above S are reported as left. (c) The reply is: FileIds that left; changed items as (final index, FileId, shown w, shown h) sorted by final index; N; set digest. The client builds a new array in a spare buffer, merging the old array (skipping left and changed ids) with changed items at their final indexes. Rank: for name order, a count of rows below the item on files_order; for keyed sorts, one pass over the scope reading (key, order key, FileId), compared with the same Rust comparator list_grid uses (compare_keys, browse/read/grid.rs:83-96), binary-searching the changed items. Above a bound (proposal: 50,000 changed items or 10 percent of N) the reply is Full.
Why: Unchanged items keep their relative order because their keys did not change; the client holds no comparator, so it cannot drift. Keyed sorts pay one scan per delta, which the desktop list already pays.
Rejected: client-side sorting from keys (drifts in WASM), per-change events carrying indexes, server-side diffs against stored state.

**Q: What do the digests check, when, and what does a mismatch do?**
Decision: Set digest: commutative sum mod 2^64 of a 64-bit hash of each FileId, maintained in O(changed), checked against the server's count and set digest on every reply. Order digest: a polynomial hash of the FileId sequence in final order, checked on every Full reply, on every reopen, and on every 8th delta (proposal). A mismatch drops the replica and requests Full, logging the listing key and both values on client and server.
Why: The set digest catches a missed membership change at the next delta; the order digest is the only check for a missed order change, at roughly one full read's cost. Until then the phone may show a wrong order but never a wrong file, because actions and records are keyed by FileId.
Rejected: order digest on every delta (a full sort per event), no order check, a cryptographic hash (a 64-bit collision is an integrity check, not a security boundary).

**Q: Which changes must the catalog stamp before any delta ships?**
Decision: A migration in the trigger house style (schema.rs:704) must make every change affecting a listing's membership, order or shape stamp files.changed (and tombstones for deletes). Verified gaps: (a) UPDATE of files.created, folder_id, name, ext and order_key is unstamped (pair.rs:355-362 take_identity; writers.rs:101 created backfill). (b) analysis UPDATE of width, height, orientation, taken, frames and duration_ms is unstamped (analysis_changed covers only version, size, mtime, rotation, kind, error, poster_ms: schema.rs:755-770). (c) The zone change rewrites analysis.taken without a stamp (handled by the epoch). Already stamped: files insert, size, mtime, rotation, missing (schema.rs:728), analysis insert and delete, ratings, border marks and sides, file writes (schema.rs:775-782), folder removal (roots.rs:58-74). A test enumerates every column in key_sql, scope, condition and entry columns (grid.rs:24-32) and asserts each is stamped or written down as not affecting listings.
Why: An unstamped change is silent divergence until a digest check. Rejected: a Rust-side touch() called by writers (misses unwired writers); polling the listing.

**Q: How do bulk changes no stamp can carry reach clients: the zone change and the layout change?**
Decision: Each writes a layout epoch in meta in the same transaction as its change. A delta whose S is older than the current epoch is answered Full. The epoch is read in the same read transaction as meta.seq.
Why: Stamping every row for a rare event costs N writes and still needs a phone rebuild; one epoch write is cheap and covers a future bulk writer. The Taken key reads the local zone at query time (query lib.rs:362-366), so the zone change reorders the list regardless of the stamp.
Rejected: stamping every row; ignoring the zone change.

**Q: Tombstones for deleted files, and how long are they kept?**
Decision: A tombstone table records each deleted files row (FileId and seq at deletion), written by an AFTER DELETE trigger on files (pair.rs:345 and 434 delete rows; no trigger covers them today). Deltas list tombstones above S as left. Retention 90 days (proposal). Full replies and each delta carry the oldest retained seq; a client older than that gets Full. A deleted FileId is never reused (decision 11).
Why: Nothing stamps a row that is gone. Rejected: keeping deleted rows marked missing forever, soft delete everywhere.

**Q: Provisional (unpaired) rows should not be listed.**
Decision: The listing condition excludes unpaired = 1. Today browse reads do not test unpaired (none under browse/), and missing defaults to 0 (lifecycle.rs:207-210), so a row a scan inserted but not yet paired appears in list_grid. A new row is unpaired until pairing at the end of its scan (writers.rs:104-111). Owner to confirm: a new file appears when its scan's pairing ends.
Why: A provisional row is not yet a file; listing it produces a membership change later undone, shown as a flicker.

**Q: files.id must become AUTOINCREMENT before any delta ships.**
Decision: Change files.id to INTEGER PRIMARY KEY AUTOINCREMENT in a layout change under the exclusive hold (lifecycle.rs:49-52). The baseline has no AUTOINCREMENT (schema.rs:502; no AUTOINCREMENT anywhere in crates/). Test: insert, delete the largest id, insert again; the ids differ.
Why: SQLite can reuse a freed largest rowid, and pairing deletes fresh rows (pair.rs:345, 434); a replica still holding the id would show a different photo. Reasoned from SQLite's rowid rule, not reproduced. Rejected: keying by (FileId, volume, file_ref) (NTFS record numbers can be reused, general knowledge), a per-row generation column.

**Q: Restart, sleep, reconnect and an interrupted apply?**
Decision: A server restart changes nothing for lists: meta.seq, stamps, tombstones and epoch are durable in WAL mode. A sleeping phone or dropped socket keeps S and asks on reconnect. A Delta is applied atomically: built in a spare buffer and swapped in with the new S only after the count and set digest (and order digest when due) pass. An interrupted apply leaves the old replica and S; repeating the same reply is a no-op because S no longer matches. A page reload drops the replica; the next open is Full.
Why: The failure classes of expiring snapshots do not exist here; the only recovery is to ask again from the held S.

**Q: Client memory: size, lifetime, persistence and location?**
Decision: The replica holds 8 bytes of FileId and 8 bytes of shown width and height per item (16 bytes per item, proposal; about 16 MB per million), with a spare buffer making the delta peak about double. At most two replicas (the one shown and one recent); the least recently shown is dropped under pressure. The replica lives in the layout worker, so the main thread receives only the visible window's data. Not persisted.
Why: The layout's decided home is the worker. Rejected: f32 aspects (not exact), a reverse FileId-to-index table (not needed by the merge), JavaScript maps or objects per item (the freeze documented at WEB_CLIENT.md:82-86).

**Q: Notices, held lists and rating edits.**
Decision: The server sends ListingChanged(listing key, S') to the owner's connections when a membership or order change commits after the listing's last delivered S. It carries no data and is coalesced to one per listing per 250 ms (proposal). The client pulls a Delta on the notice, on reconnect and on closing the viewer. A Delta arriving while the viewer is open or the user is rating is held and applied when the viewer closes (the desktop's rule, Main.qml:260-264). An attribute change (rating value, marks, rotation) goes as ItemChanged(FileId, version). A rating does not reorder the list at once; the list catches up when idle or when the viewer closes (owner to confirm; the doc's 'not a new behaviour' at WEB_CLIENT.md:224-226 does not match the desktop, which never reopens on a rating: edits.rs:75-101).
Why: A notice without data cannot be wrong in a way the pull does not fix.

**Q: The selection and scroll anchor when the selected file leaves the list.**
Decision: The selection follows its FileId. When a delta removes the selected file, before applying the delta the client takes the next surviving FileId in its old order, or the previous one if none; this uses the old order the replica holds. The scroll anchor is the row of the selected item after the delta (grid.rs:183). The desktop's current clamp of the old index to the last item (grid.rs:391-395) is replaced at cutover (owner to approve: it changes desktop behaviour; the doc's rule is at WEB_CLIENT.md:218-220).
Why: The index clamp selects whatever now sits at the old index, which is not the file that left.

**Q: Bulk actions: select all, then delete or move in version 1?**
Decision: A bulk action sends the listing key, S, N, the set digest D, the change and request_id. The server, in one write transaction, recomputes N and D over the current scope and applies only if both match; otherwise it replies ListingChanged and the client pulls a delta, shows the new count and asks again. The server keeps no selection. 'Select all on the server' (WEB_CLIENT.md:345) becomes this verified predicate. Version 1 delete sends the file to the Recycle Bin (quarantine is dropped, `WEB_CLIENT_TECH_DECISIONS.md:200`); whether a move is in version 1 is open (O40 in `WEB_CLIENT_RECONCILED.md`).
Why: The set the user saw is identified by count and digest, so an action touches that set or none of it. Rejected: a server-held selection, acting on the query without a check.

**Q: Authorisation and bounded cost on the server?**
Decision: Every listing request checks the device record's status, so revocation takes effect at the next request. Each device has one listing read in flight per listing slot; a newer request supersedes and cancels the running one through the progress-handler flag (core.rs:173-179), as reader.rs:209-238 does. A rate limit on Full replies per device (proposal 10 per minute) and a global cap on concurrent blocking reads (proposal). The read transaction ends before the reply is written, so no WAL frames are pinned (schema.rs:377), and temp_store MEMORY (schema.rs:378) avoids disk.
Why: With no server state there is no memory to cap; the expensive step is the read, so limits sit on reads. Rejected: per-IP limits (LAN devices share addresses), keep-alives.

**Q: Shape encoding for the replica.**
Decision: The replica carries shown width and height as u32 (Entry::shown_size, model grid.rs:74-76). The client computes aspect as f64 width divided by f64 height, 1.0 when either is 0 (grid.rs:79-82), clamped by clamp_aspect (client layout.rs:16-18). This withdraws the 2-to-4-byte aspect (WEB_CLIENT.md:315-316, 393-394).
Why: WEB_CLIENT.md:341 says layout matches the desktop exactly, which holds only with the same integers and the same f64 computation. Rejected: f32 (row-boundary rounding), a 2-byte log quantised aspect (lossy).

**Q: The desktop after cutover.**
Decision: The desktop becomes a client of the same protocol over loopback, with its own replica. Its in-process Vec`<Entry>` (client grid.rs:109) becomes a FileId and shape replica, or keeps Entry (open). Its reload on scan (Main.qml:127-128 and 255-264; library.rs:145-148) becomes a delta pull. Its selection rule changes at cutover.
Why: One protocol and one replica logic, tested once; the desktop's extra cost is zero because it has no network hop.

**Q: Corrections to WEB_CLIENT.md that follow from these decisions.**
Decision (once the owner approves decisions 2, 4 and 15): (1) lines 212-215: the 56-byte, 56-MB figure is the desktop's Entry; the server holds 0 bytes per listing; the phone holds 16 bytes per item (proposal). (2) lines 218-220: state the next-survivor rule. (3) lines 221-227, 302, 408 and the P4 row at 564: lifetime section replaced by no grace, idle or cap. (4) lines 205-206: stamp becomes (listing key, S). (5) lines 315-316 and 393-394: shape encoding per decision 18. (6) line 345: select-all becomes the verified predicate. (7) line 587: tombstone retention replaces event retention. (8) line 155 says SameSite=Lax; line 592 lists it as open; say one thing.
Why: Each change follows from a decision above; the 56-MB figure would size the wrong structure.

**Q: Verification before anything ships.**
Decision: (a) Model test: random interleavings of inserts, deletes (including the largest id), folder moves, renames, rating edits, rotations, border cuts, pairing rebinds and zone change, with Delta and Full replies; after each step the replica equals a Full read (FileIds, shapes, order digest). (b) Rank test: every sort key, both directions, random data with NULLs and ties; delta final indexes equal full-read positions. (c) Stamp test. (d) AUTOINCREMENT test. (e) Tombstone-horizon test. (f) A planted missed stamp is caught by a digest and logged. (g) A write committed during a Delta's computation appears in that reply or the next. (h) An interrupted apply leaves the old replica; a duplicate reply is a no-op. (i) A newer request cancels a running read.
Why: Each test is a wrong-answer path named above.

Open:

- Owner approval of decision 2 (client replica; no server per-listing state; listing stamp change).
- Owner approval of the selection rule (decision 15).
- Owner confirmation that a rating does not reorder the list at once (decision 14).
- Owner confirmation that provisional rows are not listed (decision 10).
- Desktop memory after cutover: keep Entry (about 10.6 MB for 189,518 files, dhat 2026-11-02, PROFILING.md "Measurements") or move to FileIds and shapes (16 bytes per item, proposal).
- Order digest cost and cadence at 189,518 and 1,000,000 items (not measured).
- Keyed rank cost per delta (one O(N) Rust pass per batch); an expression index would be a schema change not designed here.
- Stamp coverage completeness: production writers not fully grepped. INSERT OR REPLACE at ledger.rs:471 and an analysis update in borders_due (lifecycle.rs:139) are unchecked for other paths.
- Whether a scan's insert writes missing = 0 and whether other readers treat unpaired rows differently (only browse was grepped).
- Thumbnail cache after FileId reuse: keyed by file id with mtime and rotation (thumbcache types.rs:14-24); the row-key path not traced.
- Cascades from files deletes into stamped tables (schema.rs:113 and 204); not traced.
- Tombstone retention (90 days) and storage cost at 1,000,000 files.
- First-open and Full-reply transfer for 1,000,000 items: about 16 MB uncompressed (proposal); no compression designed.
- Android Chrome worker memory and merge time per delta (not measured).
- FileId above 2^53 is not exact in a JavaScript number; the phone's array type (Float64Array or BigInt64Array) is open.
- The layout epoch must be written in the same transaction as its change; set_mark (lifecycle.rs:90) was not traced.
- Whether events use the same meta.seq as listings (decision 2 assumes so; event design).
- Reopen and full-read cost per sort on a large library; only name-order timings exist (PROFILING.md:329, 341-342).
- SQLite rowid reuse not reproduced; the test must exist before it is called proven.
- Whether the Taken key reading the local zone can reorder unchanged rows between epochs (DST without an epoch).
- Whether a 64-bit set digest is acceptable for destructive bulk actions, or 128 bits.

Measure first:

- Listing memory per entry (56 bytes, `crates/model/src/grid.rs:41`).
- Keyed sort full read and delta timings at 189,518 items (no keyed sort or delta has been timed).
- Order digest cost; bulk action check cost; keyed rank per batch.
- Phone worker memory (16 bytes per item plus spare buffer) and merge time per delta on Android Chrome.
- Notice coalescing and refetch floors under a real scan's bump rate.

Validation notes:

- Corrections: the cold-start list is at lifecycle.rs tests (BEFORE_ORDER_KEY), not the baseline; the real baseline is schema.rs:501-529 (id at 502, missing DEFAULT 0 at 518). set_mark is at lifecycle.rs:90 (160-172 is set_taken_zone). Shrink-to-fit is at grid.rs:134-136. Client selection test is at grid.rs:599 (tests module starts at 403). PROFILING.md:329 gives 601.1 and 559.5 for two build columns; the choice of 559.5 needs its column named and 'offscreen' confirmed.
- Verified as cited: model grid.rs:14-41 and 74-82; ids.rs:10-11; view.rs:11-19; query.rs:124-137 (14 sort keys); layout.rs:16-18 and 38-53; browse/read/grid.rs 22-32, 83-96, 113-153, 156-168; scope.rs 9-20; query lib.rs 355-390 and 362-366; schema.rs 57-65, 229-231, 502-529, 535-543, 707-734, 755-782; pair.rs 345, 355-363, 434; writers.rs 101, 103-111; roots.rs 58-74; ledger.rs 471; lifecycle.rs 49-52, 139, 164-171, 639-640; controller.rs 204-205, 326, 334-337; Main.qml 127-128, 255-264; library.rs 145-148; edits.rs 75-92; reader.rs 173, 179-181, 209-238; core.rs 173-179.
- Contradictions: (1) Decision 10 makes unpaired a listing condition, but take_identity writes unpaired = 0 unstamped (pair.rs:355-362), so a file becoming listable when paired would be missed until a digest check; add unpaired to the stamp list. (2) The decided listing stamp (listing_id, generation) is replaced; this needs owner approval and must be recorded as a change to a decided item. (3) Device design's snapshot-cap decisions are void if this is accepted. (4) Auth's 'desktop' snapshot key is void likewise. (5) Rating rule and provisional-row rule are owner confirmations; approving decision 14 needs the doc's 'not a new behaviour' claim settled.

---

### real_size scope and the 1:1 control on the web viewer

**Q: What is the scope of real_size?**
Decision: Per device. In v1 only the desktop holds a persisted real_size: a field of the desktop's device record keyed by the desktop's fixed identity, so it does not wait on the device-identity key. Until the device record exists it stays a desktop field in gallery.json (prefs.rs:28-29). Phones and browsers keep no persisted real_size in v1; a new device starts with it off and it is never copied.
Why: The cost lands on the device that opens: at 1:1 the original is fetched and decoded there (about 6 MB per original, ../reference/IO.md:32; 100 opens is about 600 MB, arithmetic). Meaning is screen-relative. Rejected: per user (a phone pays for a desktop keypress), per session (regresses the desktop), server-global like write_back, per device with per-user override.

**Q: Should the web viewer show a 1:1 button, and what does it do?**
Decision: Yes. A text button beside the close control reads '1:1' when fitted and 'Fit' at 1:1 (the Yandex preview's words, YandexPreview.qml:140). Shown only for a zoomable still (viewport.rs:68-70) whose 1:1 differs visibly from fit: a named pg-client constant, proposed at 1 percent (not measured), separate from the 0.0001 reset guard (viewport.rs:171). It toggles at the viewer's centre through the same toggle as double-tap (viewport.rs:178-182). Its state is transient per image; a step returns to the fit. While full-size tiles load it shows a busy indicator; the fitted image stays visible. The HUD zoom percentage stays as the readout.
Why: 1:1 is the only way a phone user judges sharpness; on a phone the only route is double-tap, and nothing names it (keys.rs:222). The Yandex preview already ships this control. The button costs nothing until pressed. Owner to confirm the decision (it is the question the owner asked).
Rejected: no button (touch users cannot discover it); single-tap toggle (collides with overlay tap); sticky 1:1 across images (fetches tiles per swipe); a per-device off switch for the pixel path (the phone gate is the place to decide that).

**Q: Does Z exist on a phone, and what does it do in a browser?**
Decision: Z stays in the shared key map (keys.rs:133). In a browser with a keyboard, Z toggles the transient 1:1 of the current image and writes no preference. The desktop's Z is unchanged (it toggles the persisted real_size, Viewer.qml:279). The web hint reads 'Z 1:1'. A touch-only phone has no Z.
Why: The key map is shared; removing Z breaks keyboard use in browsers; a persisted browser Z would depend on the open device key.

**Q: Which hint line does a touch client show?**
Decision: viewer_hint takes an input argument, touch or keys. The touch line lists pinch zoom, drag to pan when zoomed, swipe to step or rate when fitted, double-tap 1:1 (a new string), the 1:1 button and H hide. It leaves out wheel zoom, Z, L, R and the 0 to 5 rate keys.
Why: A hint naming absent keys gives wrong instructions; the builder is the only place the text is built, so input must enter there. (Validation: the viewer hint builder is at keys.rs:294, not 234-249.)

**Q: Which phone gestures does the viewer use, and what is the swipe trap?**
Decision: Keep pinch zoom around the centroid, clamped to minimum zoom and 32x (Viewer.qml:314-320; viewport.rs:6, 155-167); drag to pan when zoomed (Viewer.qml:332-352); swipe to step or rate when fitted, 60 px threshold (:346-347); double-tap to toggle fit and 1:1 at the tapped point (:354-363); plus the button. The trap: at 1:1 a horizontal swipe pans rather than steps (:340, :349-350); the exit is the button or a double-tap back, and the touch hint says so. Overscroll none and touch-action none are proposals to verify on the phone.
Why: The gestures exist in shared QML; the trap is fixed by making the exit visible, not by changing swipe meaning, because drag-to-pan is the purpose of 1:1.

**Q: What does a 1:1 response contain: original bytes, a full-size JPEG, or tiles?**
Decision: Fixed-grid tiles of the upright image at native scale. The server decodes the original once per file version at full size (display.rs:50-54), applying rotation and pending turns (images.rs:290). Tiles are 512 by 512 image pixels (proposal), edge tiles smaller, each a lossless PNG via the `image` crate's PNG encoder (its `png` feature in `[workspace.dependencies]`). The browser draws only visible tiles (a repeater of positioned Images using viewport.rs:83-89). Tile responses are version-keyed and immutable on 200.
Why: The phone holds only the visible pixels, so memory scales with the screen (a whole 24 MP RGBA decode is 96 MB, arithmetic). Server-applied orientation is required because pending turns are catalog state (items.rs:94-101). Lossless keeps pixel identity testable. Rejected: original passthrough (wrong orientation, undecodable formats), full-size JPEG q92 (lossy, whole image in browser), arbitrary rectangles per pan, lossy tiles, on-disk full-size cache.

**Q: Between fit and 1:1, does zoom use a browser resample or the server's region render?**
Decision: Keep the server's screen-scaled region render for parity with the desktop: region/x,y,w,h/PWxPH, Lanczos3 at the window's device size (display.rs:102-115), after the 150 ms pause (ImageLayers.qml:91-95), using the DCT-scaled decode (display.rs:50-59) rather than a full decode. The web caps output at the window's device pixels plus a margin, within the view size bound in WEB_CLIENT.md (Limits: 8,192 px a side).
Why: A browser resample looks different from the desktop; the cap closes the 16384 px lever (image.rs:134).

**Q: What URL grammar do view images use on the web?**
Decision: One key grammar and one parser with two policies. The key is the desktop's ImageKey text (items.rs:41-48): file id, mtime, rotation and pending turns; the shared parser is client image.rs:119-135. Web forms: /api/files/{key}/{w}x{h} for the fitted image; /api/files/{key}/tile/{col},{row} for a 1:1 tile; /api/files/{key}/region/{x},{y},{w},{h}/{pw}x{ph} for zoom below 1:1. Fit and tile responses: private, max-age=604800, immutable (O11 option (a)). Region responses: private, no-cache. The full form is refused on the web. Replaces the sketch endpoint at WEB_CLIENT.md:398.
Why: One parser tested for both clients; the key carries the version so a rotated photo is never served under its old URL. Private, not public, because the bytes sit behind login.

**Q: What does the server do when the requested version is not current?**
Decision: The view is served only when the requested version equals the file's current catalog version. The check runs inside the consistent() read closure (images.rs:141-160), so a retry cannot decode a newer version under an older URL. Mismatch returns a typed 409 with the current version; the client refetches the item and drops old tiles. The tile grid is computed from the decoded dimensions, which must equal the catalog's upright size (items.rs:95-101); a difference is a typed error.
Why: The desktop provider ignores the version today (providers.rs:117-119); an immutable URL would pin the wrong bytes.

**Q: How is the server's full-decode cache shaped, and how are decodes bounded?**
Decision: Replace the single last_decode slot (images.rs:92, 289-301) with a byte-budgeted LRU of decodes keyed by file id, version and decode size, looked up with the covers rule. Concurrent requests for the same version share one decode (single flight). Decodes run one at a time (proposal). A decode larger than the budget is still held while it serves requests. Capped at the codec's 512 MiB (limits.rs:5-10). Decodes run on their own bounded pool. A request beyond a bounded queue gets 503 with Retry-After. Budget is a proposal.
Why: A phone pans across about 15 visible tiles of one photo, which must not re-decode; a single slot thrashes between two clients; one concurrent decode bounds memory under a burst.

**Q: How do view URLs authenticate, and what can a LAN peer do?**
Decision: Login cookie only; unauthenticated gets 401 before any decode. No token in any URL. No CORS headers on tile or view responses. A per-device rate limit on view requests (no value set). A decompression bomb costs one decode slot until the codec cap stops it. SameSite=Lax keeps the cookie off cross-site subresource loads (browser behaviour, not tested here).
Why: A token in a URL leaks into logs, history and caches.

**Q: What do failures do to the 1:1 view?**
Decision: Transient (dropped connection, 503, timeout): retry with capped backoff (proposal); the fitted image stays visible; error shown after the cap. Permanent (409 version mismatch, 404 tile out of range, corrupt original): no retry; 409 refetches the item; decode error shows an error state on the fitted image. Server crash mid-request: GETs are idempotent and nothing is written; the cache is empty after restart. Desktop crash during Z: the preference save is temp-then-rename, so the file holds old or new. Disk full: the view writes nothing; a failed preference save keeps the value in memory and posts PrefsNotSaved (prefs.rs:147-154), which must reach phone clients after cutover. Sleep: abort in-flight tile requests on hide; on wake request visible tiles only; if the version changed while asleep, drop old tiles.
Why: Nothing in flight is half-done; retrying a permanent error would loop.

**Q: Is the 1:1 view's state saved across a reload?**
Decision: No. Zoom, pan and the 1:1 flag are controller state and are not restored after a reload; the server holds no viewer state. Differs from a design that restores them from sessionStorage.
Why: 1:1 resets per image, so restoring it contradicts that rule and adds a storage path for a convenience.

**Q: Who writes the preferences file once the desktop cuts over?**
Decision: The server is the only writer of gallery.json after cutover. The desktop's settings bridge calls the server's API instead of writing the file (settings.rs:16-41). The cutover list (WEB_CLIENT.md:306-307) gains the preferences file.
Why: JsonStore::update writes its whole in-memory value and never re-reads (store.rs:51-56); two writers would revert each other's fields.

**Q: Does the Yandex preview's 1:1 control carry to the web, and does it share the viewer's button?**
Decision: The Yandex preview keeps its own 1:1 button (YandexPreview.qml:139-144) with the same words and the same visibility threshold as the viewer (the shared constant replaces its 0.01 at :169). Its single-tap toggle (:181-183) stays. Its state is per result and resets on step (:61-66), not real_size. Its full image comes from the host URL (:172), so the tile design does not cover it; its web 1:1 source is open.
Why: One control type across the app; the single-tap toggle cannot move into the viewer because of the overlay tap.

**Q: What does 1:1 cost, and when is it paid?**
Decision: Paid only when the user presses 1:1 or a zoomed view is needed. Server: one full decode per file version in the bounded cache; one PNG encode per tile (time and size unmeasured). Network: visible tiles only. Phone: visible tiles only in memory. No preload of originals for neighbours; neighbours preload as fitted images only (Viewer.qml:396-404).
Why: The honest answer to 'what does 1:1 entail': server decode and tile encode, paid once per version; the phone pays only for tiles shown. Preloading originals for eight neighbours would be about 48 MB per step (arithmetic).

**Q: Is video in scope?**
Decision: Not designed here; video stays open.

Open:

- Nothing measured: decode time, PNG tile encode time and bytes per tile, tile size 512, cache budget, concurrency of one, per-device rate limit, first-pixel time on a phone, server memory per decode.
- Pixel identity between tiles and the desktop's 1:1 pixels is a test to write; not run.
- PNG encoder speed on 512 px tiles unknown; fallback is a measured alternative, not lossy.
- Whether CSS image-rendering: pixelated matches Qt's smooth=false above 1:1 is unverified.
- Qt for WebAssembly touch behaviour (PinchHandler, touch DragHandler, TapHandler tapCount) untested; Qt 6.12.0 source not read in this run.
- The QML tile layer and GPU texture size at high zoom (512 px tile at 32x is 16384 device px) untested on Qt WASM.
- Chrome Android overscroll and touch-action unverified; check at the phone gate (WEB_CLIENT.md:610-611).
- SameSite=Lax cross-site image behaviour is standard, not tested.
- Whether decoded dimensions always equal the catalog's upright size for every supported format and EXIF orientation is unchecked.
- consistent() busy-waits on a blocking thread; how long a tile request can hold one during a long rewrite is unmeasured.
- Yandex's web 1:1 source is a host URL, not covered by the version rule or tiles.
- Web thumbnail public Cache-Control (WEB_CLIENT.md:320, 396) conflicts with the private policy for view bytes; the thumbnail design decides.
- The desktop's real_size record keyed by a fixed desktop identity assumes one desktop per server; the cutover must confirm.
- The touch hint omits B, R and L because no touch equivalent is designed.
- The VersionMismatch notice rule cited at WEB_CLIENT.md:322-324 is the batching paragraph, not the rule (open at :589).
- Whether the desktop's in-process image path stays after cutover (P5, WEB_CLIENT.md:565).
- The phone gate (WEB_CLIENT.md:610-611) decides whether the pixel path ships on phones; owner's default.

Measure first:

- Decode time per format for a 24 MP still; PNG encode time and bytes per 512 px tile.
- First-pixel time for 1:1 on a phone; tiles per screen.
- Server memory per decode and the cache budget that holds a phone's working set.
- Pixel identity tile versus desktop 1:1 (test).
- Touch behaviour on the phone gate (pinch, drag, double-tap, overscroll).

Validation notes:

- Corrections: keys.rs:234-249 is Hint::ALL; viewer_hint is at keys.rs:294. keys.rs:341-348 tests grid resize keys; the viewer hint test is keys.rs:399-411; no touch input concept exists in the shared builder, so 'both inputs' is unsupported. keys.rs:222 is Hint::TurnOn; OneToOne is 224, RealSize 225, Loupe 226.
- Verified as cited: prefs.rs 1-4, 22-29, 103, 147-154; settings.rs 16-18, 25-27, 36-41; store.rs 51-56, 66-75; Viewer.qml 60-64, 175-180, 213-226, 279, 280-281, 314-320, 331-363, 396-404; ImageLayers.qml 72-73, 91-95, 98-109, 100-102; YandexPreview.qml 24, 48-53, 61-66, 139-144, 161-184, 186-190; Hud.qml 91-92; viewport.rs 67-70, 83-89, 97-99, 155-182; images.rs 92, 141-160, 277-290, 289-301; display.rs 50-59, 102-115, 155-160; limits.rs 5-10; the `image` entry in `[workspace.dependencies]`.
- Unverified: the Resizer calls at display.rs:107 and 111 use Lanczos3; the 96 MB and 72 MB figures are arithmetic; the region output cap; Qt WASM touch and overscroll claims.
- Contradictions: (1) device gives every device kind a prefs column including real_size and browser defaults; realsize says phones keep none in v1. (2) real_size stays in gallery.json until the device record exists; device moves all prefs into server.sqlite at cutover and leaves gallery.json unchanged; auth keeps LAN settings there too. (3) The desktop real_size key: realsize says fixed desktop identity; device now sets a random 128-bit id. (4) View responses: auth says private no-store; realsize says private immutable (fit, tiles) and private no-cache (regions). (5) Thumbnail public Cache-Control question is answered by auth (private); realsize's open item is stale. (6) SameSite: realsize's argument depends on Lax; auth chooses Strict and device chooses Lax.

---

### Thumbnail transport

**Q: Which thumbnail transport for v1: HTTP image URLs, a WebSocket batch stream, or both?**
Decision: HTTP image URLs. Each thumbnail is a plain img requesting a versioned, immutable URL on the LAN listener. No thumbnail bytes on the WebSocket; no batch stream in v1; no Cache Storage in v1. The WebSocket carries events plus one small client-to-server want list. A batch stream is adopted only if the cold-fill measurement (M2) fails on both HTTP/1.1 and HTTP/2 and the event-delay test (M4) shows the shared socket is the cause. The transport works over HTTP/1.1 with six connections; HTTP/2 only raises stream count. The phone trust test and M0 gate performance, not correctness.
Why: Per-URL HTTP semantics (caching, cancel by removing the element, flow control) are what a batch would rebuild. The source the doc cites for batching, multipreview (raw ImageController.php, dbf9f4e1, lines 139-229), has no WebSocket, no cancellation, no Cache-Control and silent failures, so the doc's 'with cancellation' claim (WEB_CLIENT.md:122-123, 322-323) is unsupported. Rejected: batch stream as default (duplicates HTTP caching and puts bytes on the event socket); both transports; HTTP batching (no per-item immutable header); a fetch-based page scheduler (fallback only); copying multipreview.

**Q: What does the server answer on a request for a thumbnail that is not cached, and what does the client draw meanwhile?**
Decision: Four answers decided from cache state at request time. (1) Exact cached for (file, mtime, rotation, thumbs_version, height): 200 image/jpeg, private, max-age=604800, immutable (O11 option (a), decided by default in this pass; `WEB_CLIENT_RECONCILED.md`). (2) Exact missing but a cached height in the served set S exists for that version: 307 to that height's exact URL, no-store; the requested height is queued; the nearest lookup is restricted to S. (3) Nothing cached for that version: 200 image/png, a fixed 1x1 transparent placeholder, no-store, X-Thumb-State: pending; queued. (4) Client over its queue cap, or global cap: 503 with Retry-After, no-store, nothing queued. The request never reads an original or waits on a fill. The client draws the listing's thumbhash under the transparent placeholder, keeps a per-cell pending record, and re-requests on the ready hint or a backoff (1 s doubling to 30 s, proposal), adding a nonce the server ignores. Only the exact-height 200 is immutable.
Why: The decided rule is queue and placeholder (WEB_CLIENT.md:410-411); answer 3 keeps it with an image body so no broken-image icon shows. get_nearest_into returns the smallest height at least the request, else the largest below (lib.rs:138-172), so one URL's answer changes as the cache fills, which is why only exact-height responses may be immutable. Owner must confirm the 307, the transparent placeholder and 503.

**Q: What is in the thumbnail URL, and which heights and versions are served?**
Decision: /api/thumbs/{catalog_id}/t{THUMBS_VERSION}/{file_id}@{mtime}.{rotation}/{height} (the shape `WEB_CLIENT_RECONCILED.md` section 1.5 fixes). Turns are not in it: the client applies them in CSS, and the layout uses the turned aspect. The served height set S is a server setting published with the listing; geometric proposal 96 to 1024 with ratio about 1.25 (about 12 values, unmeasured). The client rounds row height times DPR up to a member of S. Heights outside S or outside 16..=2048 get 400 with no lookup; a missing file gets 404 no-store; a mismatched mtime, rotation or version gets 409 no-store and the client refetches the item. The desktop's unquantized heights stay in its cache but the web never serves them.
Why: The key must name the bytes it describes; turns are not part of the bytes (images.rs:219 uses current.rotation; the cache is keyed by mtime and rotation, lib.rs:149-151). Quantisation bounds cache entries and fill work. Note: thumbs_version does not regenerate existing thumbnails (reader.rs:80-87 compares only mtime and rotation).

**Q: How are cancellation and wasted work handled without a service worker?**
Decision: The server does not infer wants from request lifetimes; a miss answers at once. Each client's events socket carries an unsequenced want message: an ordered list of up to N keys (proposal 512) the client currently mounts, most important first; each new message replaces that client's previous list. One fill queue for all clients keyed by (file, mtime, rotation, version, height); a queued key's priority is its best position across clients' latest lists; ties go newest first (LIFO, as the desktop pool does, providers.cpp:4-5). A queued key no client wants is dropped if not started; a started fill always completes and is stored. A disconnected client's list is dropped. No service worker in v1.
Why: The want list is the only accurate signal and cheap. Letting every fill run wastes decodes on cells nobody sees; per-item cancel messages are a second protocol for what a replaced list already says.

**Q: How are the decode and write side bounded, and what if a made thumbnail cannot be stored?**
Decision: Viewport fills run on a fixed decode pool (proposal min(4, cores)) with two lanes: viewport first, then the background pass. Every fill waits for its store to commit (a put_batch variant, since put is fire-and-forget; lib.rs:186-205); only after commit or NotStored does the fill send the ready hint. A fill re-checks the cache before decoding. The per-client and global queue caps are the limits stated once in `WEB_CLIENT.md`; duplicate keys merge. If a store fails, the bytes are kept in a bounded recent-fill map (proposal 64 MB, 10 minutes) serving that exact key until the store succeeds or the window ends; NotStored goes to the notice channel; viewport fills pause 30 s (proposal). Touch operations coalesce into the touch set at push time.
Why: The current Pool bounds idle items only (pool.rs:18-34) and KEEP_MAKERS limits idle makers only (makers.rs:10), so a miss flood creates unbounded decoders. The writer's inbox is unbounded (writer.rs:38-61); waiting writers bound it. The recent-fill map covers a full disk, which would otherwise leave placeholders indefinitely.

**Q: What thread and connection model serves cache reads on the async server?**
Decision: ThumbCache owns a bounded reader pool of N read connections (proposal 4 to 8), checked out per read; web reads run through spawn_blocking under a semaphore of N. The thread-local READERS (reader.rs:90-109) are removed. close() drains and closes the reader pool, then stops the writer (today the writer stops first and only the calling thread's readers clear, lib.rs:307-313, so the final checkpoint can run beside open readers, writer.rs:233-238). The server's shutdown: stop accepting, finish reads, close the cache. New exact read get_exact_into for hits; new nearest query restricted to S for the 307.
Why: With thread-local readers, connection count equals threads that ever read, which an async runtime's blocking pool does not bound. The tokio default of 512 blocking threads is from memory, not checked.

**Q: Which entries may the on-demand path evict?**
Decision: On-demand eviction spares the heights some client wanted within a recent window (proposal 10 minutes), plus the height of any pass in progress. The SQL in evict changes from height != ?1 to height NOT IN (spared set), with an empty set handled explicitly.
Why: Today on-demand puts keep no height (writer.rs:255-257), so eviction fights across clients with different row sizes (writer.rs:400-407).

**Q: Is Cache Storage used?**
Decision: No, not in v1. The browser HTTP cache holds thumbnails under the immutable exact-height URL. Revisit only if M7 shows the HTTP cache discards thumbnails the phone needs across sessions.
Why: img cannot read Cache Storage without a service worker or a blob copy; Cache Storage quota on Android Chrome unverified; a second cache with no reader is two stores to keep consistent.

**Q: Authentication and cross-site safety for the thumbnail GET, which enqueues work.**
Decision: The login cookie is httpOnly SameSite=Lax (WEB_CLIENT.md:155; the value is then settled, closing :592). A cross-site img carries no cookie and gets 401 with no work. The route checks authentication before any cache or queue work. If Sec-Fetch-Site is present and cross-site, 403 and no enqueue (defence in depth). Responses carry no catalog paths. Thumbnails are never public. Each response sets Cross-Origin-Resource-Policy: same-origin.
Why: A GET that enqueues decode work is a cross-site amplifier if the cookie is sent. The doc's Origin check covers the WebSocket only (WEB_CLIENT.md:412-413).

**Q: Logout, device revocation and what the browser's HTTP cache keeps.**
Decision: Logout responds with Clear-Site-Data: "cache", "cookies", "storage". On 401 the client clears pending records and timers and returns to login. Revocation fails the device's next request, closes its socket and drops its want list. Residual, accepted and stated: an HTTP cache entry on a revoked device can be shown from disk until its max-age ends or it is evicted. Max-age is a constant, not a setting.
Why: Shortening max-age would cost the caching the design depends on and still leaves the window open. Clear-Site-Data's effect on an open page is unverified.

**Q: Sleep, reconnect, restart and crash.**
Decision: On visibility to visible, re-apply src on mounted cells with a nonce (re-issuing requests the browser abandoned), re-send the want list, and resume the event socket with resume(seq). Completed thumbnails stay in the HTTP cache. Server restart loses the queue and recent-fill map; pending cells re-request on backoff. A build change reloads the browser (WEB_CLIENT.md:262-264). Server crash loses derived work only; thumbnails are made again. Each put is one transaction (writer.rs:151-166), and SQLite recovers the WAL at next open.
Why: Browsers freeze or fail backgrounded requests, so in-flight state is stale on wake; the cache holds derived data that originals rebuild (config.rs:5-8). Rejected: resuming in-flight requests, persisting the queue.

**Q: Connection limits, streams and the TLS dependency.**
Decision: Set SETTINGS_MAX_CONCURRENT_STREAMS explicitly (proposal 128). HTTP/1.1 with six connections works if HTTP/2 is not negotiated, since a miss answers at once and a hit is one read. The event socket takes one of the six HTTP/1.1 slots (open whether WebSockets count). Whether the TLS library negotiates HTTP/2 on Windows is decided in the TLS area; this design needs only that the server can speak HTTP/2 when asked.
Why: The stream cap bounds one connection's work; the library default is not checked against the pinned version.

**Q: Which doc statements must change?**
Decision: WEB_CLIENT.md lines 122-123 and 322-323 (multipreview has no socket, cancellation or cache headers); 319-322 and 396-397 (the URL is private and carries thumbs_version, not immutable for a nearest height); 324 (no Cache Storage in v1); 325-326 (plain img works with the 307 and placeholder rules); 383-387 (native-tls 0.2.18 has a server ALPN accept path under alpn-accept, with a chain FIXME, to be restated); the cited 422-423 does not exist (file is 617 lines; the rule is at 410-411); 592 (SameSite Lax).
Why: The doc is the decision record and must not disagree with it.

Open:

- Chrome on Android trust gate (P7) not run; the design depends on it only for HTTP/2 performance.
- Whether Chrome cancels an in-flight img when its element is removed or src cleared; Blink source not read.
- Whether a no-store placeholder is reused from memory cache on the same URL (decides the nonce; measure M5).
- Whether a ready hint can arrive before the placeholder response is processed (race; frequency unmeasured).
- Whether Chrome reloads after Clear-Site-Data 'cache' and 'cookies' on logout.
- Whether an open WebSocket counts against Chrome's six HTTP/1.1 connections, and whether axum's WebSocket upgrade works over HTTP/2 (RFC 8441).
- native-tls 0.2.18 server ALPN on Windows: the accept path exists (schannel.rs:334-366) and the chain FIXME is at schannel.rs:360; runtime negotiation unverified; alpn-accept is not enabled in the `ureq` entry in `[workspace.dependencies]`; the private CA chain delivery is the real risk. TLS choice open.
- Hyper's SETTINGS default and tokio's default of 512 blocking threads are from memory; confirm against the lock once the server crate is added.
- Served height set S: values, ratio, and whether desktop heights should be quantised too.
- Whether the background pass should fill the phone's heights in S (the pass runs one height at a time, thumbs lib.rs:173-179).
- Reader pool N, decode pool size, queue caps, want-list size, recent-fill map, NotStored pause, backoff, 128 streams: all proposals.
- The 307 and placeholder refinements and the 503 need owner confirmation.
- Thumbhash delivery shape (WEB_CLIENT.md:591 not investigated).
- Yandex pictures from external hosts are not covered; a proxy must not forward credentials or cache under this origin's rules.
- Desktop after cutover reading over loopback: Qt per-host connection limits on loopback unverified.
- Revocation residual (HTTP cache until max-age or eviction) accepted and named, not solved.
- Cache Storage quota and eviction on Android Chrome unverified.
- Thumbnail sizes at phone heights unmeasured; only 29.6 KB mean at 384 px (THUMBNAILS.md:34).
- Cache disabled (ThumbCache::disabled, lib.rs:115): the web path needs a defined answer; not designed in full.
- Whether 'pending' thumbnails should be visible to the owner.

Measure first:

- M0: h2 negotiated on the phone with the trusted certificate; server logs ALPN.
- M1: wasted bytes during a fling of 3 screens per second for 5 seconds on about 5,000 items, cold cache; proposed pass at most 25 percent wasted; p95 removal-to-want-list under 200 ms (all proposals).
- M2: time to a full 60-cell screen at 220 CSS px and DPR 3, for h2, HTTP/1.1 with six connections, and the batch stream if built; 20 cold, 20 warm; report p50 and p95.
- M3: 128 concurrent cache hits through the reader pool at N in {2, 4, 8}; latency, CPU, threads created.
- M4: miss path with an empty cache: placeholder response time (proposal under 50 ms at p95), zero original opens on the request path, fill-commit-to-hint time, race frequency.
- M5: placeholder re-set with and without the nonce on Android Chrome.
- M6: 10,000 distinct misses from one client: queued count stays under the cap, RSS bounded, remainder get 503; 10,000-key want list size cap.
- M7: 1,000-thumbnail listing across a browser close and reopen: refetched versus disk-served.
- M8: full or read-only cache volume with 60 misses: thumbnails display from the recent-fill map until TTL; one NotStored notice.
- M9: mean JPEG size at each height in S (only 384 px exists: 29.6 KB mean, THUMBNAILS.md:34).
- M10: Clear-Site-Data on Chrome Android with the page open.

Validation notes:

- Correction: the native-tls FIXME is at schannel.rs:360 (the accept starts at 353; ALPN request at 364-366). The 338-368 range is accurate as a range.
- Verified as cited: get_nearest_into 138-172 with ORDER BY at 151; put and put_batch 186-205; close at 307-313; reader.rs 80-87 and 90-108; writer.rs 233-238, 255-260, 357-364, 400-407; images.rs 30, 141-161, 185, 214-219; thumbs lib.rs 173-179, 274; providers.cpp 4-5 and 105; render lib.rs 10-14; size.rs 12 and 71-80; image.rs 12-18 and 45-49; pool.rs 18-34; makers.rs 10; WEB_CLIENT.md 122-123, 155, 319-322, 396-397, 410-411, 412-413, 592.
- Not verified: Chrome image cancellation and placeholder reuse; tokio blocking-pool default; hyper stream default (neither package is in Cargo.lock); the M-series (none run); Win32 chain behaviour on accept.
- Contradictions: (1) The want list and resume(seq) depend on a WebSocket the events design removes; the events design has no upstream channel for the want list (only POSTs). (2) Desktop unquantized heights (decision 5) conflict with the cutover's rule that the desktop never writes the thumbnail cache and that the server queues makes; either the server serves unquantized heights to an authenticated desktop or the desktop keeps a second writer. (3) Cache-Control: this design says private; events D16 says public and puts turns in the path with the render version in a query parameter. (4) The ready-hint and backoff mechanism conflicts with events' thumbs scope (D10/D16); the events design has no hint channel.

---

### Desktop cutover: moving reads and writes together

**Q: Scope: which desktop operations move in the one release?**
Decision: Every desktop path that reads or writes catalog.sqlite, thumbs.sqlite, gallery.json or the inbox moves in one release. (a) Catalog rows: rate, border mark, turn, add, remove and rescan folder, job start, pause and scan requests, session listing save, saved views save and delete, inbox post, mark, dismiss, clear, retry and viewing. (b) Files written: crop copies (save_crops), the file writer's XMP and turn copies, the Yandex save. (c) Thumbnail cache: keep, forget, height, viewer keep. (d) gallery.json: prefs_effect, write_back and real_size settings, Yandex folder record. (e) Startup and shutdown: prepare (migrations, rebuild), thumbnail cache open and start-over, inbox start, unreadable-prefs quarantine, last-file flush, library stop, catalog close. Reads that move with them: session, views, roots, grid, tree, facets, entry, details, failed files, file paths for players, images. Video's device source and the native folder picker stay in the desktop process (video not designed).
Why: The server is the only writer after the cut, so any row left in the desktop is a second writer. Preferences move because JsonStore rewrites the whole file from its own copy (store.rs:51-74).
Rejected: writes first and reads later (two processes on one catalog); moving the catalog but keeping prefs or the inbox in the desktop; a forked write implementation.

**Q: How is the desktop's data path kept reversible while the cut stays one switch?**
Decision: One desktop backend trait with two implementations: Direct (wraps pg_app, current code) and Loopback (HTTP client to the server routes calling the same pg_app code). All app::get call sites move to the trait before the flip. The flip is one default change in the desktop entry (crates/desktop/src/lib.rs:164 today). One conformance suite runs against both.
Why: A reference implementation for the parity suite and an in-tree rollback. The trait's output types live in client or model crates, not pg-app.
Rejected: rewriting each call site straight to HTTP; a RemoteApp whose methods block on the server.

**Q: Landing order inside the one release, and when may the server first run?**
Decision: Commits, each passing the gate alone. C1: the server singleton and the catalog guard; the in-process desktop and CLI direct writes take the singleton for their whole run, so no second writer starts beside them; a second in-process desktop is refused (a visible change). C2: the server crate with routes for every inventory operation, unused by any user path, run only from tests. C3: discovery, token, handshake and loopback hardening, tested. C4: the Loopback implementation; the conformance suite runs against it. C5: the event stream replaces the in-process change bus and inbox subscription, with an explicit resync on gap (see contradictions). C6: prefs, views, inbox and the thumbnail placeholder move behind the trait. C7: CLI routing. C8: the flip (default Loopback; the desktop drops its pg-app dependency; tools/crate-layers.txt and the boundary check change together). C9 (a later release): logon autostart with its own gate.
Why: The earlier design's S1 would open pg_app while the Direct desktop still writes (two writers). The server runs only user-started after C8.
Rejected: a dark-launch server beside the in-process desktop; starting the logon task before the flip; switching reads and writes in separate releases.

**Q: Singleton lock: what is it keyed on, and what does it protect?**
Decision: The server and every in-process writer (the Direct desktop, a CLI write run) hold <catalog_dir>/server.lock, taken with a non-waiting try (a new variant in pg_dbfile; Lock::take waits about 5 s and is wrong for an instance check). Keyed on the catalog directory, not the home folder; server.db sits in the home (B1 in WEB_CLIENT_TECH_DECISIONS.md) and the launch file launch.json sits there too (B9).
Why: The unit with one writer is the catalog; two homes can share one catalog directory (layout.rs:29-44). The OS releases the lock at process death, so a crash leaves no stale lock.
Rejected: a home-keyed lock (two homes, one catalog); a named mutex (per user, not the catalog); a port-based singleton.

**Q: Catalog-level guard against old builds, the CLI and a server starting beside a holder.**
Decision: Split Catalog::prepare into a public prepare that takes the prepare lock and an internal body the server calls while holding catalog.sqlite.lock for its whole life. At startup the server takes server.lock, then catalog.sqlite.lock (held for life), then a non-waiting Live::exclusive probe on catalog.sqlite. Any other shared holder makes the probe fail; the server waits a bounded time, then fails with a message. After startup the server's own shared holds are the only holds. CLI reads use Catalog::open, which takes no prepare lock (core.rs:133-136).
Why: The prepare lock is released when prepare returns (core.rs:67-69), and the exclusive hold is needed only when a change is needed (core.rs:91-95), so an old build with an unchanged schema could write beside a server without this guard.
Rejected: relying on the singleton alone (old builds don't take it); relying on the shared hold alone (doesn't stop a same-schema opener).

**Q: CLI modes: which commands go to the server, which open directly?**
Decision: Writing commands (scan, remove, import, thumbs, borders, write): if server.lock is held, an API call with the launch token, reporting job progress and treating Ctrl-C as cancel. If not, the command takes server.lock for its whole run and runs today's direct path. Read commands (status, dups): Catalog::open if the catalog is at this build's version; otherwise take server.lock and run today's prepare. The CLI never calls prepare while a server runs.
Why: Today every command calls open_catalog and prepare before dispatch (cli lib.rs:136), and prepare runs mark_unheld_missing (core.rs:115), which starts a write, so a read is a writer today.
Rejected: CLI always calling prepare; refusing CLI writes while a server runs (decided against); always going through the API (a CLI with no server must still work).

**Q: Attach: how the desktop finds, starts or replaces the server.**
Decision: (1) Probe server.lock with a non-waiting try. If held, read launch.json (home: port and token), connect to 127.0.0.1:`<port>`, call hello; if protocol and build id match, attach. (2) If free or discovery fails, spawn the same binary as 'gallery serve' with explicit --home and --catalog-dir, detached with no console, using a plain Command with detach flags, not spawn_background (process.rs:22-27 puts the child in a kill-on-close job). A spawn race loser exits at the lock and both attach to the winner. (3) On a mismatch call the old server's authenticated stop route, wait for the lock to free within a bound, then start the new one; an old build without a stop route shows a message naming the process. Never kill. (4) Port 0 recorded in launch.json after bind. (5) The window shows a connecting state with the reason in place of the fatal CatalogUnopened exit (desktop lib.rs:164-166). The GUI thread never blocks on attach.
Why: The lock is the only truth about whether a server is alive. Discovery files left by a crash are treated as absent.
Rejected: trusting launch.json without the lock probe; a desktop-owned server that never attaches; killing an old server; the job-object spawn.

**Q: Loopback token, listener and hardening.**
Decision: 32 random bytes from the OS source at each start, written with its port to `<home>`/launch.json (temp-then-rename, B9), with a DACL granting only the current user (new Windows code). Every route except hello needs Authorization: Bearer, compared in constant time. Listener binds 127.0.0.1 only; rejects Host other than 127.0.0.1 or localhost with the server's port; rejects any request with an Origin; no CORS; no cookies. hello returns protocol version, build id and catalog id only. The LAN router never accepts the token; the loopback router never accepts a login cookie. Token never on a command line, in an environment variable or a log. launch.json is removed at clean stop.
Why: A bearer header is not CORS-simple; WebSocket upgrades are not CORS-checked, so the Origin rejection is needed. Two routers keep trust levels apart.
Rejected: token in a cookie or query string; a per-launch token in the registry; a named pipe with its own ACL; accepting a LAN cookie on loopback.

**Q: Layout changes and the exclusive hold mapped onto the server.**
Decision: A layout change (prepare's migration or rebuild, cache start-over and move-aside) runs only in the server's startup critical section, before the listeners bind, while holding the singleton and after a successful probe. The server never takes the exclusive hold while running. A layout change needed while running is refused with the in-use error. A layout change needing a new build is done by stopping the old server through its stop route, then starting the new one. No in-process quiesce. A catalog file absent while catalog.sqlite.replaced exists makes the server refuse with a notice before prepare runs.
Why: The rebuild moves the catalog aside (rebuild.rs:87-88) then renames the replacement into place (rebuild.rs:89-91); a crash between leaves no file, and prepare then rebuilds an empty catalog without warning (core.rs:70-74, 97-98). The refusal closes that case.
Rejected: in-process quiesce (many handles, no gain over exit); runtime layout change; letting prepare create an empty catalog when a .replaced file exists.

**Q: Schema and rollback neutrality of the cutover.**
Decision: The cutover makes no change to the catalog's schema. Server-only state (request ledger, device and login records, event log, last job report per request) lives in `<home>/server.db` under its own version. The per-item mutation version is a separate later step with its own migration and gate.
Why: A catalog with a newer schema is refused by an older build (core.rs:75-80); any cutover-time migration makes the pre-cutover binary unable to open the catalog, removing the binary rollback. The desktop grid has no version field (model grid.rs:16-45).
Rejected: per-item version migration in the cutover release; login or device tables in the catalog.

**Q: Mutation semantics and retries: rating, border mark, rotation and other non-set operations.**
Decision: Rating and border mark stay absolute, unconditional, last-writer-wins; the desktop sends no expected_version in the cutover. Turn is a server command turn(file, by) adding turns to the pending total inside the single writer. Every non-set operation (turn, crop copies, Yandex saves, folder add and remove, scan and import requests) carries a client request id. The server writes the outcome to server.db's ledger in the same SQLite transaction as the catalog write (server.db ATTACHed to each catalog connection). A retry with a known id returns the recorded outcome and applies nothing. If the attached transaction fails the gate, the fallback is at-most-once turns with an unknown state after a lost reply, decided by the user.
Why: Turns are relative (queue_rotation adds modulo 4, queue.rs:103-120), so a retry without an outcome turns the file twice. The client does not hold the pending value, so absolute turns from the client race.
Rejected: client-computed absolute turns; a catalog table for the ledger; a compare-and-set on the pending value (cycles modulo 4).

**Q: Optimistic edits, lost replies, and exit while a mutation is unacknowledged.**
Decision: Rate, mark, rotate and viewer flags apply in the desktop at once. On rejection the change reverts and the existing edit-failed notice posts. On a lost reply a set is resent (idempotent) and a turn is resolved by its ledger outcome, never resent without one. The desktop does not exit while a sent mutation is unacknowledged, within a bounded wait; on timeout it shows the write as unknown. Unsent mutations are dropped with a notice if the window closes.
Why: The write is synchronous today (edit.rs:27-30), so closing cannot lose one; over HTTP the reply can be lost after commit.
Rejected: waiting for the server before showing the change; fire-and-forget.

**Q: Reads and the UI thread.**
Decision: No loopback call runs on the Qt thread. Every read returns through a deliver callback, as the reader does (reader Session::submit). The startup reads (saved view, session listing, preferences, roots, write_back and real_size) become asynchronous, and the window shows a connecting state until the first snapshot. The window keeps its own copy of settings and updates it from events.
Why: Today those reads are in-process and cheap (desktop controller.rs:39-55); over loopback a stalled server freezes a synchronous window.
Rejected: synchronous reads with a timeout; a new async runtime in the desktop.

**Q: Thumbnails: the miss path and the cache's single writer.**
Decision: In the Loopback backend the desktop never writes the thumbnail cache. A miss returns the ThumbHash placeholder and the server queues the make (images.placeholder, images.rs:269). The desktop replaces the placeholder only when the server announces the made thumbnail for that file at that height; a placeholder is never stored as final. The Direct backend keeps make-on-read until the flip; the difference is a listed exception in the parity suite. The server makes thumbnails at the heights clients want through a keyed queue.
Why: Today the desktop makes thumbnails synchronously and puts them in the cache (images.rs:173-260). Two writers to one cache would break its start-over and cap rules.
Rejected: keeping make-on-miss in the desktop; caching the placeholder as final; last-writer-wins height (two clients thrash; each change stops the pass, runner.rs:1-12).

**Q: Preferences, saved views, session listing and the inbox: ownership.**
Decision: The server is the single owner of gallery.json (existing JsonStore, same format, no migration) and of the catalog rows for views, session listing and inbox. Whole-file rewrites are correct with one owner, so no per-key patches are added. The desktop flushes its pending preferences and listing through the server before disconnect, within a bounded time. The inbox's viewing flag becomes a per-client set; the inbox counts as viewed while any client reports it. Link notices (server unreachable, attach failed, timeout) are transient and shown in the window only. The desktop's change pump and inbox subscription move to the server's event stream with resume on reconnect and explicit resync on gap (contradicted; see validation).
Why: With two writers prefs would lose updates; the singleton excludes that. A latest-wins viewing flag is wrong with two clients (inbox.rs:23, 431). The in-process bus loses the oldest events for a slow subscriber (library.rs:157-160).
Rejected: a desktop-local inbox or prefs file; per-key patches; a global viewing flag.

**Q: Logs while both processes exist.**
Decision: The server writes gallery.log, the desktop gallery-desktop.log and the CLI gallery-cli.log, all in the data folder. Log rotation is not built yet; the planned rotation is pg-log's capped rotation (proposal: 5 MiB by 3 files, `WEB_CLIENT_RECONCILED.md` 5.16).
Why: The log module documents a loss when another instance appends between copy and empty (log file.rs:17); separate files remove it without changing the format.

**Q: Yandex save and search after the cutover.**
Decision: The save moves to the server. The desktop keeps the native folder picker and sends the chosen folder; the server normalises the path with its own rule, saves through the same save code, and records the folder through the preferences owner. The server also serves the search.
Why: The save records the folder in gallery.json (yandex.rs:533-550), so it writes prefs and must be server-owned. One implementation for desktop and phone.
Rejected: Yandex save in the desktop (a second file and prefs writer); phone saves naming any folder (open).

**Q: Paths clients send (folder add, crop save, Yandex save, export).**
Decision: The loopback API accepts a path only as a value the server normalises itself, using the CLI's rule (pg_platform::names::normalize_path). The server never trusts a path the desktop normalised. The LAN router applies the same rule and the folder-browser restriction for library roots.
Why: The same API later serves the LAN, where a client path is untrusted.

**Q: Background job, file writer, and the server's lifetime.**
Decision: The job, file writer and inbox start with the server. The desktop's Library starts and stops nothing; it sends requests. The desktop starts the server only when attach finds none, and restarts it if it dies while the desktop is open. Desktop exit never stops the server; the server stops on its stop route, console close or Windows shutdown. The logon task is a later step with its own gate and explicit arguments.
Why: Phone scans must not depend on a window. Today the job starts with the window's Library object (desktop library.rs:87-110).
Rejected: a job running only while a window is attached; a Windows service (different session and token file location).

**Q: Startup repair and crash recovery.**
Decision: At startup, under the guard, the server (a) deletes every scan lease, since only the server writes and it holds the guard; (b) refuses the rebuild-window state; (c) leaves writer temp files alone, as they are the only whole copy of a write (writeback lib.rs:1-20); (d) treats the launch file as absent unless the lock is held. Passes resume from the catalog (runner.rs:1-12). Queued inbox notices are lost on a crash, as today. The ledger makes a retried command safe.
Why: The scan lease is 60 s (types.rs:89) with an unchecked pid holder (roots.rs:221-223), so after a crash a scan would fail as busy for a minute. Rejected: waiting out the lease; checking pid liveness; deleting temps.

**Q: Shutdown order.**
Decision: Desktop: flush preferences and listing through the server, close the connection, exit after any bounded wait for unacknowledged mutations. Server: stop accepting; close phone and loopback sessions; stop the job and writer at a file boundary; drain the writer and inbox within bounds; close the pool and the keeper (the keeper closes last, since the last connection deletes the WAL); remove the token and the launch file (`launch.json`); release catalog holds; release server.lock last.
Why: Matches the existing order (app.rs:148-160); releasing the singleton last means no other process starts a writer while the server closes.

**Q: Rollback: lever, procedure and what it leaves untouched.**
Decision: Two forms. (1) In-band, one release only: the Direct backend in the same binary, selected by an environment variable. Direct takes server.lock, so it refuses while a server holds the singleton. The release after removes it. (2) Binary rollback, valid because the cutover is schema-neutral: stop the server through its stop route, confirm server.lock is free, disable the logon task, run the pre-cutover build. Both builds open the same catalog. gallery.json is rewritten by the server on stop, so it holds current values; server.db is ignored by the old build.
Why: The singleton blocks dual-code writes while a server runs. Design rejected: restoring the pre-migration backup (loses all post-cutover writes); a permanent Direct mode (re-admits a window-owned writer).
Note: the file is gallery.json (layout.rs:37-38), not prefs.json as the input said.

**Q: Test gate before the flip (C8).**
Decision: On native Windows, then the Linux pass in the pg-dev container (the CI check; no WSL): (1) just test; (2) just qmltest; (3) just shots against the reference; (4) conformance suite on both backends (versions, sequence numbers, resume, snapshot-generation mismatch); (5) parity suite: the same operation sequence on Direct and Loopback from identical fixtures, comparing catalog rows, file_writes, notices, views, gallery.json, cache rows and events row by row, with only listed exceptions (the thumbnail miss); (6) cross-process tests (the other_process.rs pattern); (7) a crash matrix with kill -9 at named points (after a temp is recorded, mid-copy, mid-rebuild, mid-scan, mid-turn, mid-inbox, mid-thumbnail batch, mid-Yandex save), checking no double copy, no double turn, no unfinished temp; (8) rebuild-window test; (9) auth: no token 401, wrong token 401, Origin and foreign Host refused, LAN refuses the token, token DACL checked; (10) version skew both ways; (11) two desktops, desktop with CLI, server killed mid-attach, token rotated under a running desktop; (12) rollback rehearsal on a copy of a real catalog; (13) baseline timings before the flip: listing open, 100-item range fetch, placeholder time-to-fill, attach time, startup with migration and with rebuild.
Why: Each item targets a failure named above. Budgets are set from measurements, not chosen first.

**Q: Crate boundaries and the desktop's dependencies after the flip.**
Decision: At C8 the desktop drops its pg-app dependency (desktop Cargo.toml); the boundary check and tools/crate-layers.txt change in the same commit (the desktop line is 'desktop: app client log model platform render trace', crate-layers.txt:48). The trait's types live in client or model, not pg-app. The CLI keeps library and catalog for the direct path under the singleton.
Why: A dead pg-app dependency would make the boundary check prove nothing.

Open:

- OWNER DECISION (amends a decided item): turns are server commands with request ids, not client absolute values with expected_version.
- OWNER DECISION: whether the atomic ledger write (server.db attached to the catalog connection, one transaction) holds in this build. Believed from SQLite multi-database transaction semantics; not verified, including under WAL and the SETTINGS batch. Fallback: at-most-once turns.
- OWNER DECISION: whether the in-band Direct lever ships for one release (recommended: yes, time-boxed).
- OWNER DECISION: during C1 to C7, a CLI write while an in-process desktop is open: refuse (recommended) or allow.
- OWNER DECISION: a second desktop process is refused from C1 (the hazard is two processes; the premise that two windows each start a Writer is wrong, see validation). Confirm the interim behaviour.
- OWNER DECISION: per-item mutation version migration ships in its own release before optimistic edits need it, or after the phone.
- OWNER DECISION: thumbnail height policy for several clients (largest height requested in a window, and the window length).
- Logon start mechanism and restart policy are decided (O5); the open part is whether creating the task needs elevation.
- OWNER DECISION: scope of real_size (WEB_CLIENT.md P1) must be settled before preferences move (C6).
- NOT VERIFIED: Windows DACL for launch.json; no DACL code exists in the repo.
- NOT VERIFIED: breakaway from a job object for the spawned server if the desktop was launched in a kill-on-close job (installer or launcher).
- NOT VERIFIED: Windows LockFileEx semantics between two handles of one process (the exclusive probe runs before the server opens its own handles).
- NOT VERIFIED: that Connection::open creates a missing catalog file (core.rs:71 relies on it).
- NOT VERIFIED: the rebuild-window reasoning end to end; needs a test before the refusal is trusted.
- NOT VERIFIED: the Windows firewall prompt for a loopback bind (believed none).
- NOT VERIFIED: sleep and hibernate with stale or held locks.
- OPEN, outside this area: event retention and resume semantics; the server epoch (the change pump depends on them).
- OPEN: the Yandex folder a phone may name, and whether a phone save is allowed in version 1.
- OPEN: log rotation; whether two processes appending to their own files is loss-free.
- OPEN: an old desktop window open during an upgrade: the server's start refuses; wording and recovery not designed.
- OPEN: same-user malware reads the token, catalog and originals; the token is not a boundary against it.
- OPEN: a WSL or other-OS process writing the same catalog; the lock holds only within one OS (lock.rs:17-19).
- OPEN: video (device source stays in the desktop; no design).
- OPEN: whether the pg_app blocking pool sized to catalog connections covers the viewer's synchronous full-image decode under concurrent use.

Measure first:

- Listing open time, 100-item range fetch, thumbnail placeholder time-to-fill, attach time with the loopback server, startup with a migration and with a rebuild (backup copy at core.rs:105 copies the whole file), and the time a CLI or desktop waits on the singleton.
- Cost of the attached ledger transaction per turn against a plain write; ledger size growth and retention under scan-rate event volume.
- Bounded wait for unacknowledged mutations at exit; handshake and attach bounds; connection pool size and KEEP_CATALOGS; free-space threshold before a layout change (all proposals).
- Thumbnail height window and placeholder visibility on a first view of an unmade grid.

Validation notes:

- Corrections: every CLI command calls open_catalog at cli lib.rs:136 (inside run_command), not 122. The 'second desktop window' open question's premise is wrong: Library::start is per process (app library.rs:257-262) and the desktop returns early when running (desktop library.rs:101-103), so one process has one Writer. The hazard is two desktop processes; restate the question. The rollback file is gallery.json (layout.rs:37-38), not prefs.json. Cited lines core.rs 69, 91-95, 101, 105, 115, 123, 133-137; lifecycle.rs 40-48 and 49-52; lock.rs 12-13; layout.rs 29-44; app.rs 148-160; edit.rs 27-30; inbox.rs 23 and 431; library.rs 155-160; log file.rs:17; process.rs 22-27; desktop lib.rs 164-166; types.rs 89; roots.rs 221-223; tools/crate-layers.txt:48 verified. Unverified: rebuild.rs 87-91 are close but not exact; core.rs 75-80 newer-schema refusal; Windows lock and DACL claims.
- Contradictions: (1) C5 and the open list assume resume and gap resync and a server epoch; events D9 removes the event sequence, ring and resume entirely, replacing them with a change vector. Adopt the vector and drop the resume and gap text. (2) Attach rule requires protocol and build id; events D4 drops the general protocol integer and gates on build id equality; pick one. (3) Schema neutrality is false once quarantine's migration 10 or the viewstate version columns ship: pre-cutover binaries then refuse the catalog (core.rs:75-80). No design reserves migration numbers or orders them. (4) Preferences: the cutover keeps whole-file JsonStore writes and a flush; viewstate requires per-field versioned writes and says the desktop never writes a whole Prefs after cutover. (5) Turn: cutover uses relative server commands with request ids; viewstate uses absolute set with expected_version. (6) Rating and border mark: cutover sends no expected_version; viewstate requires it. (7) Ledger: cutover's server.db ledger covers quarantine commits, while quarantine keeps its request id in a catalog table (quarantine_batches.request UNIQUE): two idempotency stores. (8) Placement: cutover says trait types live outside pg-app; events D21 puts wire types in pg-wire (depends on pg-model); amend one rule. (9) Thumbnail cache: cutover says the desktop never writes it after Loopback; thumbs decision 5 keeps desktop-height writes.

---

### Quarantine (dropped)

Noted 2026-11-21 in this pass: quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`). A user delete sends the file to the Windows Recycle Bin, and no purge command exists from any client. The quarantine design that stood here (the `_quarantine` store, held rows and their recovery, the purge rule, the operations, tests and build order) is removed from this file.

### File ids (migration 10)

**Q: Are file ids reused?**
Decision: Migration 10 makes files AUTOINCREMENT through a table rebuild that keeps ids (copy_tables, rebuild.rs:181-210). Ids are never reused. Fallback if the rebuild cannot run with foreign keys on: a minimal tombstone row with volume and file_ref NULL, missing = 1, the cascade removed.
Why: files.id has no AUTOINCREMENT (schema.rs:502), so a freed largest id can be reused. Clients identify items by FileId, so a reused id shows another photo. AUTOINCREMENT also fixes the existing pairing deletes (pair.rs:345, 434).
Rejected: leaving reuse as it is; tombstone as the primary design (rows grow and need identity nulling).

Open:

- Whether a migration step can rebuild files with analysis, ratings and file_writes referencing it with foreign keys on (schema.rs:36-44; rebuild.rs). Not read; the tombstone fallback applies if not.

Measure first:

- AUTOINCREMENT table rebuild time on a large catalog.

---

### View-state split and control/data split

**Q: P8 premise: is there a member table in WEB_CLIENT.md to fix?**
Decision: No. The doc has no per-member table; the only mention is the P8 row (WEB_CLIENT.md:568), which describes the gap, and P8 is 'Proposed (M)' (lines 553-570). The table is generated from the bridge source and committed as a manifest, checked by a lint, not hand-written into the doc.
Why: A hand-written table is how the gap arose; a generator plus a count check (272 members; every attribute classified) prevents silent drift. Attribute counts over crates/desktop/src/*_bridge.rs: qproperty 112, qinvokable 140, qsignal 20, total 272, matching WEB_CLIENT.md:429-431. The 40 #[inherit] lines are Qt base calls, outside the 272.
Per file (props + invokables + signals): shell 56, gallery 36, yandex 26, viewport 23, filters 20, notices 18, library 17, command_line 17, picker 13, folders 9, failed_files 7, keymap 6, facet_choices 5, log_model 5, words 4, animation 3, video_source 3, trace 2, settings 2 (sum 272). Seven bridge files declare no QML type: deadline, frame_sink, gpu_device, gui_app, providers, screenshot, video_player.

**Q: Which members does the P8 premise name, and what is failed_files?**
Decision: The three video_source members the premise names are attach(player) (video_source_bridge.rs:24-25), play(file_id) (:27-30) and stop() (:32-34). failed_files declares seven members: qproperty total, loading, error (failed_files_bridge.rs:27-31); qinvokable data, role_names (roleNames), row_count (rowCount) (:38-50); qinvokable show(pass) (:63-64); begin and endResetModel are #[inherit] (:52-58). Which one the doc omitted cannot be determined without a table; the seven listed are the complete set.
Why: The premise is checkable only against a table. Naming show() as the omitted member would be a guess.

**Q: Stub coverage.**
Decision: Correct the doc's '9 of the 19' (WEB_CLIENT.md:441) to 8 of 19. The stubbed types are AnimationPlayer, GalleryModel, KeyMap, Settings, Shell, VideoSource, Viewport and Words (stubs/pg/gallery/qmldir:2-9). VideoLog (qmldir:5) is a test singleton with no bridge type. Eleven bridge types have no stub.
Why: The qmldir lists eight bridge types plus one test singleton.

**Q: The member classes and provisional labels.**
Decision: Five classes plus video. C control (controller-backed; crosses the protocol as events): provisional 103. D data (catalog, thumbnails, jobs, Yandex; crosses as data-layer calls): 142. P device (native handles that never cross: picker native/open-in-app/choose-command-line, trace first_frame and ready, AnimationPlayer setSink and playing): 7. T test-hook (CommandLine's 17 properties read by TestHooks.qml:68-79): 17. Qt-internal: the 40 #[inherit]. V video (attach, play, stop): 3, open, not designed. Labels come from names and module headers and are recorded as provisional.
Why: The two-class split (WEB_CLIENT.md:337-338, 431-433) cannot place device members or CommandLine; placing device members as control would make the browser build depend on native handles. Video is open, so its members stay open.

**Q: Which Commands cross the wire?**
Decision: Event and Command stay in-process (they use indices; WEB_CLIENT.md:200-208). A separate typed wire enum is converted in one place, the client data layer. Of the 21 Command variants (client controller.rs:144-192), wire-bound: OpenList, Rate, Turn, ToggleBorderMark, SetBorderMark, RefreshTurned, SaveSession, SaveLastFile, SaveDevMode, SaveShowBorders, Size::SaveRowHeight, and Size::ThumbHeight as a parameter of the thumbnail read. Local only: Select, RestoreSelection, Reveal (they select in the UI), Started, Step, ShowViewer, Relayout, Panel, OpenPanel, ClosePanel, Size::RowHeight.
Why: Corrects the earlier design: RestoreSelection and Reveal send nothing to the server. Each write is named so write-path tests can target it.

**Q: How do synchronous QML data reads meet the asynchronous protocol?**
Decision: QML data members read only the data layer's in-memory cache, synchronously. Network data becomes a request whose result arrives as an event. No QML member blocks on the network. Index-carrying commands convert to FileId at the data layer using the snapshot the controller last showed (ListShown generation); a command with a stale generation is refused with a typed error and re-sent. Rate, Turn and ToggleBorderMark become absolute set(field, value, expected_version) operations; ToggleBorderMark no longer computes !marked at execution (shell.rs:486-487).
Why: FailedFiles.show is already asynchronous with a delivered callback (failed_files.rs:144-151), so it maps unchanged. FIFO between OpenList and Rate holds only on the desktop (gallery.rs:61-64; shell.rs:474-478), not on a browser channel.
Rejected: making every QML member async; per-member RPC; a generic call(name, json) (loses typed members and crosses JSON on the GUI thread).

**Q: The desktop controller's reads at start.**
Decision: All four reads move out of Core::new into a StartSource trait returning the Start before Controller::new: argv options (desktop controller.rs:38), the saved view for --apply-view (:39), the session listing (:51), prefs (:55). The failure branches (:41-48) stay as notices. A failed session read produces Start.session_known = false with a visible notice and a retry; it never saves a default. PG_RECORD (:57) stays desktop-only.
Why: Verified by reading: after a failed read, Listing::default() is what the next save writes, because CurrentChanged arms SaveSession (client controller.rs:291-292) and fire() writes self.listing and SaveLastFile once started (:532-535). A selection change alone could overwrite a stored session without user action.
Rejected: blocking the GUI thread on a server read; a cached Start on disk (a stale cache overwrites a newer session); quarantining the session row (JsonStore quarantine applies to JSON files only).

**Q: What does ViewState hold?**
Decision: One serde struct in pg_client (controller/view_state.rs), with each field's scope: listing (remembered, per user; the session row, versioned compare-and-set); row (remembered, per device); dev_mode (remembered, per device); show_borders (remembered, per user); last_file (remembered, per user); current (session, per viewer); viewer_open (session); filters_open (session); scroll: optional anchor {file, fraction} (session, per viewer; Fraction a validated newtype in [0,1] rejecting NaN and infinity); format (u32, the browser copy's format). Start becomes {view, saved_row, overrides, session_known, apply_view}; StartOverrides holds the run-only options (--query, --sort, --size, --borders, --dev, --filters) and the applied view. Only the view is saved.
Why: One scope per field, matching the doc's preferences lines (WEB_CLIENT.md:188-198). Keeping overrides separate fixes a verified leak: Controller::new takes the start listing with options (client controller.rs:218) and SaveSession writes self.listing (:533), so a --query becomes the saved session on the first selection change after Started, contradicting the Start comment (:34-36).

**Q: What is excluded from ViewState, and why?**
Decision: Excluded: initial (controller.rs:210); StartGate (start.rs:36-47); Deadlines (deadlines.rs:33-34, absolute now_ms, no serde); generation and reveal (controller.rs:204-208, request ids and a pending action bound to one list); grid_width (controller.rs:203); GridSize.scale and shown_thumb (size.rs:40-44, device measurements); saved_row (a start input); Input.count (derived from the snapshot); the snapshot listing_id (data layer only); the modal panels (panels.rs:79-80) in version 1 (open); apply_view and StartOverrides (run-only); the desktop filter bar's Counts (display data).
Why: Each is a request id, timer, device measurement, start-only value or a value derived from a snapshot the server holds. Restoring modals would show data read before the reload and race the start gate.

**Q: Where does the Listing spec sit, and is snapshot identity in ViewState?**
Decision: Listing stays in pg_model::view (view.rs:11-19). ViewState.listing is the session listing, whose only persistence is the server's session row. ViewState carries no snapshot identity; the server issues a random listing_id per OpenList (never reused), and the controller's generation counter (controller.rs:204-205, 334-337) is the single request id the server echoes.
Why: crate-layers.txt:12 has client depending only on model and app has no client dependency (:46), so Listing cannot move into pg_client. A persisted snapshot id can name an expired list, and a reused id could show a different one.

**Q: Mid-start snapshots and resume.**
Decision: No ViewState is captured or restored before Started. The start is redone from server prefs, session and StartOverrides (a pure function, desktop controller.rs:91-118). Session-only fields (current, viewer_open, filters_open, scroll) apply as an overlay after Started and after the list is shown for its generation: current selects its FileId if present, otherwise the selection moves per WEB_CLIENT.md:218-219; the scroll anchor is reapplied the same way. The overlay never touches the gate.
Why: The gate's flags refer to a list that may have gone (start.rs:44-50), so restoring them could skip OpenAll.

**Q: Persistence path for each part of ViewState.**
Decision: The client never writes ViewState as one blob. Remembered fields are written as per-field versioned commands to the server. Session-only fields may go to sessionStorage, per tab, as a versioned whole-or-nothing copy, with try/catch on every access and discarded on format or build mismatch. localStorage is not used (shared across tabs). The desktop persists no ViewState. The server never stores session-only fields.
Why: sessionStorage is per tab; these fields are per-viewer conveniences as WEB_CLIENT.md:188-189 reserves. Whole-object writes lose updates across devices (store.rs:51-57).

**Q: Validation on read.**
Decision: Two policies, one parser. The session-only copy is whole-or-nothing: any parse, format or invariant failure discards it and the start proceeds from the server. A server-remembered field is validated on its own read: a failure falls back to that field's default and posts a notice. The wire Listing is strict: an unreadable date or unknown key is a typed error, not a dropped filter. The catalog's stored read keeps lenient behaviour with a notice. One Query/DateRange parser returns the value and the dropped fields; the wire path rejects a non-empty list; the catalog path posts it.
Why: DateRange lenient_day drops an unreadable date and so drops the filter silently (query.rs:199-201, 211-214), and Query and DateRange are serde(default) (:197, 217-219). Wrong listings must not reach the phone silently.
Rejected: strict Listing for stored sessions at once (breaks stored data without migration); field-by-field fallback for the session copy (can apply a truncated listing); lenient wire parsing.

**Q: Selection and scroll identity.**
Decision: ViewState.current is an Option`<FileId>`. Input.current stays an index local to one snapshot (input.rs:37-40); the data layer converts using the snapshot. The clear rule (input.rs:70) stays. Scroll is an anchor: the FileId of the topmost visible item (GalleryGrid.topItem(), GalleryGrid.qml:41-45) plus the fraction of that row scrolled past. QML reports the anchor when scroll rests, through a controller pause (300 ms proposal, unmeasured), never per frame or as pixels.
Why: An index is valid for one snapshot; pixel offsets break on resize; the desktop model already re-anchors by file (gallery/model.rs:179-201). Note: the doc says the controller owns scroll (WEB_CLIENT.md:192) but the code keeps it in QML (GalleryGrid.qml:42, 123); here the controller holds the anchor and QML keeps rendering.

**Q: Serde compile.**
Decision: Expected to compile; not compiled (no build run, by instruction). Field types already derive Serialize and Deserialize by reading: Listing (view.rs:11), FileId (ids.rs:9-11, transparent), RowHeight (size.rs:15-17), bool and Option. Fraction is new and needs a try_from derive. The Start derives (controller.rs:37-38) round-trip through the replay Init record (replay.rs:16-21). Not to be derived: StartGate, Input, Panels, GridSize, Deadlines, Controller.
Why: The derive needs only the field types to implement the traits, which by reading they do.

**Q: Format and replay.**
Decision: Start changes shape, so the replay format (REPLAY_FORMAT_VERSION, replay.rs:13) bumps from 3 to 4. The existing refusal of another format's recording (replay.rs:63-65) applies; format 3 recordings are not read by format 4.
Why: Init embeds Start, so a changed Start is a changed recording shape.

**Q: Sleeping phone and server snapshot expiry.**
Decision: ViewState stores the spec, not the snapshot. On reconnect the client reopens from ViewState.listing; on `snapshot-expired` it reopens the same spec and reapplies the scroll anchor and selection by FileId. Time is an input, so a pause due on wake fires at once (replay.rs:4-5; Event::Timeout controller.rs:136-137).
Why: The snapshot is a cache of the spec's result; the spec recreates the same order when the catalog is unchanged.

**Q: Remembered writes: outcome and failure.**
Decision: Every write returns a typed outcome: Applied(version), AlreadyApplied, VersionMismatch(current), NotPersisted(reason; value kept in memory), Refused(validation). A lost acknowledgement is resolved by re-reading: an equal value counts as applied; otherwise the client refetches and shows a notice. The server acknowledges only after the store's sync succeeds, and a failed write removes its leftover temp file. Current write_atomically (store.rs:68-75) neither removes the temp on failure nor syncs the directory.
Why: Disk-full is not a version conflict; each failure needs a different response.

**Q: Pending saves when the page dies.**
Decision: SaveSession (1000 ms, deadlines.rs:25-26) and ApplyQuery stay debounced. Send Flush on visibilitychange to hidden and on pagehide, which runs waiting pauses (controller.rs:138-139). Session saves are sent at once while the socket is open. A bounded loss of up to 1 s of last-file and session changes is accepted when Android kills the page without an event.
Why: A pending timer is lost with the page; the loss is bounded and stated. Removing the debounce would save on every arrow key.

**Q: Session row failure at start: the save rule.**
Decision: A save writes the session listing only if the start read it (session_known) or the user changed the listing in this run. A selection change alone never writes a listing the start did not read. SaveLastFile is a separate command, independent of the session.
Why: Closes the hazard at client controller.rs:291-292 and 532-535.

**Q: The poisoned-stub run that gates any bridge rewrite.**
Decision: In this order, before any bridge is rewritten: (1) generate the member manifest from the attributes, every member classified, count checked against 272 (the generator rejects an attribute it cannot parse); (2) generate a QML stub for each of the 19 types from the manifest, where every member throws and records its name (qmltest.py stages the stubs, qmltest.py:48-60); (3) a test that instantiates Main.qml and runs the start until Started and ListReady (no test does today; the only Main reference is a comment, tst_viewer.qml:121); (4) record the touched set before Started; this set becomes the Start payload's contract. Plus: (a) a Rust data-layer stub whose every method counts calls, with control-only members leaving the count at zero; (b) a stub whose session or prefs reads fail, where no SaveSession, SaveLastFile, SaveDevMode or SaveShowBorders is emitted; (c) a stub that delivers ListShown for an older generation after a newer OpenList, then a Rate: the rated FileId must be the one shown when the key was pressed; (d) a Rust fake of the FFI seam for the non-QML bridge files (about 18 functions in 8 files, approximate).
Why: Existing stubs return values, so unexpected reads pass silently (VideoSource.qml 5-13 and Shell.qml 5-11 are plain stubs; Shell has one property and one function against 56 members). The QML harness cannot see the FFI.

**Q: The wire types: session and prefs writes.**
Decision: Per-field versioned writes on the server (from migration 10; until then unversioned, O17), one record per scope: per user (show_borders, last_file, the session listing with its version); per device (row_height, dev_mode, yandex_folder); real_size open. The desktop never writes a whole Prefs after cutover, and the cutover moves reads and writes together, so the server is the only writer.
Why: Whole-value writes from a stale copy revert another device's change (prefs.rs:147; store.rs:51-57). The session row uses INSERT OR REPLACE (catalog listing.rs:50), last-writer-wins, so compare-and-set needs a version.
Rejected: whole-object writes serialised on the server; a per-device session (contradicts the decided per-user scope).

**Q: FileId reuse under deletes.**
Decision: Open for sequencing; the requirement is fixed. Before any delete or move ships, files.id must never be reused (AUTOINCREMENT or equivalent), and every mutation naming a file checks identity. A persisted FileId (last_file, current, scroll anchor, a Rate in flight) must not name a different file after a delete.
Why: files.id has no AUTOINCREMENT (schema.rs:501-502) and pair merges delete rows (pair.rs:345, 434).

**Q: Restarted desktop.**
Decision: The desktop persists no ViewState. A restarted desktop rebuilds its Start from its reads (before cutover) or from the server (after), shows a connecting state until Start arrives, and does not fall back to defaults for a save.
Why: The server survives the desktop; a desktop-side cache could overwrite a newer session.

Open:

- Whether phone and desktop share one session listing (WEB_CLIENT.md:188-198 says it follows the user); ViewState.listing and the session CAS depend on it.
- Whether real_size is per device or per user (P1).
- Whether the viewer is restored after reload (proposed no for version 1).
- Whether modal panels should be restored after reload (proposed excluded; owner to confirm).
- Whether a Start override may never reach the session. The owner must confirm the current behaviour (a --query saved on the next change) is not intended.
- Sequencing of the files AUTOINCREMENT migration (migration 10) against the cutover.
- Whether a Query strictness change on the wire breaks any stored saved view or session with an unreadable date; needs a count over real catalogs.
- Whether Format 3 recordings must stay readable after the bump; the count is not measured.
- Whether sessionStorage survives page kill and tab discard on Android Chrome, and how often discard happens.
- Whether the bounded 1 s loss on Android page kill is acceptable (owner product decision).
- Scroll settle pause (300 ms, unmeasured); the Fraction representation and rounding; grid layout code not read in full.
- Whether the 40 #[inherit] declarations are invisible to QML (cxx-qt source not checked).
- The AnimationPlayer's class (labelled open with video) not verified.
- Member labels provisional; the generator must check that every attribute starts its own line.
- Directory fsync after rename in write_atomically is absent; whether NTFS needs it for durability is unverified.
- Not verified: the serde compile; replay and QML tests passing; the poisoned-stub run; rowid reuse on the real catalog; the session-read failure hazard (by reading only); Android page-kill and sessionStorage behaviour; Start overrides in a real run.

Measure first:

- The poisoned-stub run's touched set (becomes the Start payload contract).
- Serde compile of ViewState and Fraction.
- Count of stored sessions and saved views with unreadable dates (Query strictness).
- Count of format-3 recordings in use.
- Scroll settle pause and anchor fraction on a phone.
- sessionStorage survival and discard frequency on Android Chrome.
- Time to fill the grid after resume.

Validation notes:

- Correction: the Command::Started push is at client controller.rs:366; the apply_view SaveSession is at 363-364.
- Verified: the 272 count, 112 qproperty, 140 qinvokable, 20 qsignal; per-file counts; 40 inherit; 21 cxx_override; 19 files with qml_element and 7 without; video_source_bridge.rs:24-34; failed_files_bridge.rs 27-31, 38-50, 63-64; Event and Command both have 21 variants (client controller.rs:76-140 and 144-192); Start derives at 37-38; FORMAT 3 at replay.rs:13; desktop controller.rs 38, 39, 51-53, 55, 57 and start_state 91-118; client controller.rs:218 and 291-292; fire() 531-534; deadlines.rs:25-26; GalleryGrid.qml:41-45 and 122-123; Main.qml:89 and 194; qmldir 2-9; size.rs:15-17; listing.rs:50; store.rs:68-75; prefs.rs:147-156; views.rs:34-38; query.rs DateRange serde(default) with lenient_day at 211-213; WEB_CLIENT.md:441 and 568.
- Not verified: member bodies (labels provisional); the serde compile; Format 3 recordings; the Start payload in a real run; replay.rs:16-21; sessionStorage on Android; Shell.qml's stub range is 6-11 (trivial).
- Contradictions: (1) Turn: viewstate requires absolute set with expected_version; cutover uses a relative server command because the client does not hold the pending value (grid.rs has no version field; rotation cycles modulo 4 at queue.rs:103-120). Both cannot be the write path. (2) Rating and border mark: viewstate requires expected_version; cutover sends none. (3) Prefs: viewstate requires per-field versioned writes on the server; cutover keeps whole-file JsonStore writes and flushes whole preferences. (4) Schema: the versioned session row and versioned prefs need a version column, which cutover's schema-neutral rule forbids in the cutover release; no store is named for the version. (5) SaveSession rule (save only after start read, or on a user change) must be carried into the cutover's session-listing write, which does not mention it.

---

### Handshake, build-id reload and event stream

**Q: Failure inventory: every route to a wrong answer, and the decision that closes it.**
Decision: Stale bundle talking to a newer server: closed by the build check on every request before any handler (decision 2), build_id and bundle_id (3), reload with hop guard (6), no-cache entry and hashed assets (7). Stale page sending a mutation whose meaning changed: refused with 409 before the handler runs (2); the page stops sending (8). Stale page reading a changed shape: the build check also covers reads (2); frozen envelopes are versioned (4). An undecodable response on a build-gated route is a stale signal, never skipped (4, 8). Restarted server reusing numbers: no sequence numbers exist; the epoch makes any restart a full re-read (9). A gap hiding a missed change: no gap can exist, since the client compares its vector with the server's current vector (9, 10). An older frame or fetch overwriting newer state: monotonic adoption and per-record versions (12). Ring overflow or slow-consumer loss: no ring and no per-client queue (9, 11). Coalescing losing information: coalescing only merges pending bumps into the current vector (10). Ambiguous set retried and applied twice, reported failed though applied, or lost on reload: pending record, reconcile by reading, no automatic resend (17). Another device's rotation leaving an old layout: the shape scope (10). Immutable thumbnail serving old pixels or caching a placeholder for a year: complete key and no-store placeholder (16). Cross-site page or DNS rebinding reaching data: Origin on every POST and Host allowlist on every request (19). Token or cookie in a URL or log: no URL credentials (19). Revoked device still reading: revoke and expiry wake held polls and every request re-checks the device (11). Login expiry read as an outage: a poll's 401 is an ordinary status (13). Reload loop: hop guard (6). Thundering herd after restart: full jitter backoff (13). Half-open connection shown as live: per-poll watchdog and visibility re-polls (13). Same-protocol older build attached: build_id equality (15). Event flood starving state: the vector is small and never queued behind bulk data (14, 16). Not closed (see open): an out-of-band writer bypassing the lock; a server commit that changes data without a bump; a reload that never settles under constant churn.
Why: The change-vector design removes whole classes that a sequence-and-ring design must close one by one (sequence reuse, gaps, replay overlap, ring overflow, resync races, replay caps). Tests alone cannot cover orderings; the generated tests in decision 23 supplement the mechanisms.

**Q: What does a client check before it trusts a response, and in what order?**
Decision: (a) The LAN has no handshake route. The login page checks its own bundle_id before the password field and reloads a stale page; a stale or failed check never blocks login. The loopback `GET /api/hello` returns {envelope, build_id, catalog_id, proof} and sits outside the LAN gate (`WEB_CLIENT_TECH_DECISIONS.md` B9; `WEB_CLIENT_RECONCILED.md` 1.2). (b) Every other request passes one middleware in this order: 1 TLS and Host allowlist; 2 the pre-login routes (GET /login and POST /api/auth/login) skip the rest; 3 session cookie or loopback token, else 401; 4 device not revoked, else 401; 5 POST only: Origin equals the configured origin, else 403; 6 the declared build identity (X-Pg-Bundle for the browser, X-Pg-Build for desktop and CLI) equals the server's, else 409 build-changed with the frozen envelope and no handler runs; a request declaring neither gets 426; 7 the route. (c) No socket and so no upgrade gate.
Why: The build check covers reads as well as writes because a same-protocol read can carry changed meaning. Rejected: a check only at upgrade (HTTP writes bypass it); a protocol-only check on reads; a version in the URL path (splits caches).
Note: step 2 exempts the login POST from the Origin check (step 5), which contradicts decision 19; see validation.

**Q: What is the build identity, and how is each side's comparison computed?**
Decision: Two identities, each a BLAKE3 digest (`blake3` is a workspace dependency). build_id: a digest of the content of every Rust input the gallery binary compiles (all workspace crates' sources and manifests, and Cargo.lock), excluding QML and the bundle's own files, computed by a build script in pg-wire with rerun-if-changed. The server, desktop and CLI share one build_id because the gallery is one binary with a serve subcommand (WEB_CLIENT.md:268). bundle_id: a digest of the bundle manifest (every served file name and content hash), written by the bundle step; index.html carries it and is outside the digest. The browser's requests and reload guard use bundle_id; the desktop's attach and CLI use build_id. At start the server checks the manifest's build_id against its own and every file's hash; a mismatch refuses the LAN listener, keeps loopback, and logs.
Why: A source digest gives an exact comparison without a random per-build value; the browser's bytes are covered by bundle_id; splitting the two keeps desktop QML live reload (live_reload.rs:1-6) from restarting the server. Rejected: a random bundle id as the desktop-server identity; git SHA (dirty trees, builds outside git); CARGO_PKG_VERSION (every crate inherits the one `[workspace.package]` version, 0.1.0-dev; used in one log line, desktop lib.rs:144); timestamps; a digest of the built binary (cannot embed itself); one combined id (restarts the server on each QML edit).

**Q: Is there a protocol version integer, and how is wire compatibility kept honest?**
Decision: No general protocol integer. Only a frozen set carries envelope 1: the handshake reply, the login reply, and error bodies (401, 403, 409, 426, 429) with codes (not-authenticated, origin-refused, build-changed, version-mismatch, rate-limited). Golden JSON fixtures for those types run in pg-wire tests, following the notice golden test (notice.rs:1379-1392). Every other response is build-gated, so no build reads another's shape. Wire enums carry no serde other and no defaults on required fields, so an unknown value is a decode error, which on a build-gated route is a stale signal.
Why: Once every handler is behind the build check, a hand-bumped integer guards nothing new and can be forgotten. The repo already refuses other formats by version (replay.rs:11-13, 46-47).
Rejected: a protocol integer on every type (unnecessary); no version at all (a stale page could not parse its own 409 body).

**Q: What does a running server promise about its build and bundle?**
Decision: A running process serves one build_id and one bundle_id for its whole life. At start it reads the bundle manifest and bundle files into memory; a rebuilt directory cannot change what it serves. A change needs a restart; the server never hot-swaps. The memory cost of holding the 27.7 MB wasm is unmeasured.
Why: Every response comes from the build the process started with, so no state inside the server can disagree with the bundle.

**Q: How does a stale page reload, and how is a reload loop prevented without storage?**
Decision: On a stale signal the page navigates to index.html?b=`<server bundle_id>`&n=`<hop>`, where n counts automatic reloads in this chain. An inline check in index.html runs before any bundle loads: if its own bundle_id differs from b, the entry is stale despite no-cache, so it shows a static update-failed message with a link to the bare URL and does not reload again. Otherwise it may reload automatically while n is below 3 (proposal). No localStorage, sessionStorage or cookie.
Why: The URL is the only state that survives reload without storage (storage can be blocked in private windows). The inline check stops a loop even when the app code is the stale part.

**Q: How is a stale cached bundle prevented?**
Decision: No service worker in v1. index.html is served no-cache with an ETag, revalidated on every navigation. Every other bundle file has a content-hashed name, served public... (see below): max-age=31536000, immutable and private behind login. The bundle step renames the fixed names the Qt emitter writes for the loader and wasm (open), so any build change changes every immutable URL. A future service worker caches only hashed names, never index.html, and deletes other bundle_ids' caches on activation.
Why: The entry point is the only mutable thing and is always revalidated; immutable names change with their bytes. A service worker would be a second update path with its own lifecycle, and v1 needs no offline start.
Rejected: a service-worker precache (keeps the old shell); a long max-age on index.html; query-string versions of one fixed name.

**Q: What does a client do when it sees a stale signal?**
Decision: A stale signal is a 409 build-changed, a bundle_id mismatch on the login page or index, or an undecodable response on a build-gated route. The page: (1) issues no new set and no new poll; (2) waits up to 3 s (proposal) for sets in flight; each gets an answer, and a 409 build-changed means nothing was applied; (3) reloads by decision 6, or, if a set is still in flight, shows an updating banner and reloads when it answers. Pending records (decision 17) survive the reload.
Why: A stale page sending sets is the one way it changes data; stopping first keeps each set's outcome known.
Rejected: reloading at once (outcomes become unknown); a banner only (the stale page keeps writing); continuing until the next poll.

**Q: How is the change stream numbered and retained? What does resume replay, and what is a gap?**
Decision: No event sequence, log, ring, retention window, replay or resume endpoint. The server keeps a current generation (u64, from 0 each epoch) per scope (decision 10). The client's state is a change vector: the epoch plus the generation of each scope it holds. The server compares the client's vector with its current one; scopes with a higher generation are re-read. The epoch is 128 random bits from the OS source, drawn at start and never persisted (the `getrandom` package is already in Cargo.lock, so declaring it adds no package). A changed epoch means every scope moved. Full reload is needed only when a listing snapshot is gone (SnapshotExpired, or an epoch change), when the page is stale, or on explicit refresh. No 'too far behind' case and no gap: a missed change is a generation the client does not hold yet.
Why: Replay logs add an epoch, retention numbers and a replay cap, and still need the full-refetch path, so they add a second path. Catch-up cost is bounded by scopes that moved, not by missed changes. The repo already has this shape: jobs events.rs:127-140 says a Change 'says only that something changed'.
Rejected: a sequence with a 4,096-event ring and 512 to 1,024 replay cap (every number a guess); a persisted sequence (an outbox table); per-item change history.

**Q: Which scopes exist, what bumps each, and what does a client do when one moves?**
Decision: tree: a scan commit changing folders or files, folder added or removed, delete or move. items: any item attribute commit (rating, mark, turn, scan-written rows; scan rows must bump, see open); the viewport and the viewer's up to eight neighbours (WEB_CLIENT.md:352-353) are revalidated on a move; other held records revalidate when they come into view. listing:`<id>`:membership: a commit adding, removing or reordering a member of an open listing under its query and sort, including scan, delete, move and quarantine, and a turn under an aspect-ratio sort (query.rs:141-145); evaluated per commit for open listings only; the client reopens with its query and keeps the selected FileId and scroll anchor (WEB_CLIENT.md:218-219). listing:`<id>`:shape: a turn on a member; the client re-reads that listing's ratio column and re-runs layout. thumbs: the thumbnail cache gained entries, so placeholders can be replaced (decision 16). inbox: notice post, replace or clear, or poke (notice_sink.rs). jobs: scan, import and writer status, coalesced to one bump per second (proposal). settings: write_back and per-user preferences. A bump happens only after the commit it describes.
Why: Each scope carries the cheapest signal its action needs. The shape scope answers rotation without per-item ratio patches, which would need a FileId column the ratio blob does not have (WEB_CLIENT.md:315-316). A turn can change membership under an aspect sort, so the doc's query-and-sort rule must cover it.
Rejected: one global generation (every rating reopens every listing); per-item ItemState frames (needs the FileId column); a per-item version map in every poll; treating a turn as no layout effect.

**Q: How does the server hold a poll, avoid a lost wakeup, and bound what one device can hold?**
Decision: A tokio watch holds the current vector, updated after each commit. A poll subscribes first, then compares the client's vector; if different it answers at once, otherwise it waits for a change or the hold timeout (proposal 25 s), answering with the current vector. A change is held up to 250 ms before answering so bumps batch (proposal). At most 2 held polls per device (the limit in `WEB_CLIENT.md`, Limits); a third gets 429 with Retry-After. Revoke, session expiry and server stop wake the device's held polls at once. Auth runs when the poll starts.
Why: Subscribing before comparison closes the window where a change lands between comparison and wait. Waking on revoke makes revocation take effect at once. The held-poll cap is the one stated in `WEB_CLIENT.md`.
Rejected: a per-client server queue (overflow or forced close); a broadcast with no per-device cap; checking revocation only at the next request (a held poll could outlive the revoke by a full timeout).

**Q: What do a fetch and a refetch that race, and what makes the result the same in either order?**
Decision: The client reads the generation first and the data second; the server bumps after commit, so a change is either in the data or reflected in a later bump. A scope's generation is adopted only after that scope's refetch succeeds, and adoption is monotonic. Every fetched record carries its version and is applied only if newer. A listing response carries its membership generation and is discarded if older than the newest seen for that listing. Overlapping refetches are allowed. Refetch rounds at least 250 ms apart (proposal). A VersionMismatch on a record read at the expected version, with no items bump since, is logged as a diagnostic with both versions, as evidence of a missed bump.
Why: Without the per-record version a slow older refetch overwrites a newer one. The diagnostic keeps the one failure the design cannot close visible.
Rejected: pausing refetches during reload (widens the stale window and feeds livelock); subscribing before reading (no subscription exists).

**Q: What is the client's sync state machine, and how do timers, backoff, 401 and liveness work without a socket?**
Decision: A pure state machine in pg-wire with no clock. States: Idle, Polling, Refetching, Backoff, LoginRequired, Stale, Stopped. Inputs: a poll's answer (vector, status, Retry-After), network error, watchdog, visibility, online, now_ms; timers and HTTP sit in the transport. Backoff: exponential with full jitter, base 500 ms, factor 2, cap 30 s (proposals), reset on the first successful answer. Liveness: each held poll sets its transfer timeout to the hold timeout plus a margin (proposal 10 s); the Qt wasm network path passes that to emscripten_fetch (qnetworkreplywasmimpl.cpp:276). A 401 stops polling and shows login at once; a poll returns its status, so no probe is needed. A network error or 5xx backs off and marks data not live. A visibility or online event aborts the held poll and polls at once. A refetch round that does not advance the adopted vector backs off.
Why: Without a socket, a dead connection is a request that outlives its timeout and a 401 is a readable status. This removes the WebSocket lifecycle, close codes and probe. A clockless machine can be tested (controller.rs:1-6 makes the same choice).
Rejected: a WebSocket with an application heartbeat (browsers can't send ping, and close codes hide the 401); a probe after three failures; fixed-interval polling; immediate retry.

**Q: What is the transport for change notices?**
Decision: POST /api/changes is a long-poll carrying the client's vector. Commands stay POSTs (WEB_CLIENT.md:400). No WebSocket or SSE in version 1. Thumbnail images are plain HTTP. The long-poll wants HTTP/2 on the LAN listener so it does not hold one of six HTTP/1.1 connections per host (WEB_CLIENT.md:113-114); without HTTP/2 it still works at one connection per client. This changes WEB_CLIENT.md:322-323 and 403-404 and needs owner approval.
Why: The state is a vector, so one answer is complete; the server keeps no per-client queue; every failure is an HTTP status. Qt for WASM loads a response body wholly into memory (qnetworkreplywasmimpl.cpp:259), so SSE cannot stream. The Qt wasm WebSocket has no ping API (qwebsocket_wasm_p.cpp), and the browser WebSocket API gives no status on a failed upgrade (WHATWG, not checked here).
Rejected: a WebSocket with a bounded queue and resume channel; SSE; interval polling.

**Q: How does the desktop use the same mechanism over loopback, and when does it attach to a running background server?**
Decision: The desktop uses the same vector and gates over loopback. The per-start launch token travels in an Authorization header, never in a query string. Host must be 127.0.0.1 or localhost; a request with Origin is refused. The desktop attaches to a running background server only if its build_id equals the desktop's. Otherwise it asks that server to stop with the token, waits, and starts its own server. It replaces a server once per desktop start; a second mismatch is an error state. The jobs Change bus stays inside the server process, where it bumps scopes; the desktop has no change source of its own. This changes WEB_CLIENT.md:262-263, which checks protocol only.
Why: Both processes are one binary, so equality is exact for a new server. The attach rule matters for a background server left by an earlier install. A same-protocol server can carry different controller and write logic, so the protocol check alone is unsafe. Once per start stops two builds replacing each other in a loop.
Rejected: attaching on protocol match; killing the old server without a graceful stop (an in-flight set becomes unknown); a desktop-only in-process change signal.

**Q: How are thumbnails named, cached and replaced when the cache misses?**
Decision: The thumbnail URL names its pixels exactly: /api/thumbs/{catalog_id}/t{THUMBS_VERSION}/{file_id}@{mtime}.{rotation}/{height} (the shape `WEB_CLIENT_RECONCILED.md` section 1.5 fixes, as at line 691). Turns are not in it: the client applies them in CSS. THUMBS_VERSION is the render constant, bumped on any change to resampling, encoding or the cache schema. A rendered thumbnail is private, max-age=604800, immutable (O11 option (a), decided by default in this pass; the owner may change it). A miss queues the work and returns a placeholder with no-store, so it is never cached under the immutable key. The client learns a placeholder can be replaced from the thumbs scope and re-requests the images it showed as placeholders.
Why: Only a complete key makes an immutable max-age correct: the URL names the source file's version (mtime and rotation) and the render version, and the cache keeps no record of the filter or version that made an entry (THUMBNAIL_RESAMPLING.md:290-292). The no-store placeholder prevents an immutable placeholder staying on the phone indefinitely.
Rejected: a shorter max-age; a build id in the thumbnail URL (every release discards the thumbnail cache); polling each placeholder.

**Q: How does a set behave across a lost response, a reconnect, a reload and a stale page?**
Decision: A set is set(field, value, expected_version) with the build header. Before the request leaves, the client writes a pending record to IndexedDB (file, field, value, expected_version, time) and deletes it at a final answer. Answers: applied (new version; delete record); 409 version-mismatch with the current version and value (adopt current if newer, cancel the item's queued sets built on the old version, post VersionMismatch, delete record); 409 build-changed (nothing applied; decision 8); no answer (the record stays). On reconnect or start, a surviving record is reconciled by reading the item: version expected+1 with the sent value means applied; version expected means not applied, so the page says the change was not saved and does not resend; anything else shows the current value with VersionMismatch. A record is never resent automatically. If IndexedDB is cleared, no reconciliation is possible.
Why: A read gives the outcome without a second write, and the user is told when a change did not apply. Rejected: retry on reconnect with a value comparison (writes without asking, reports nothing); a server-side idempotency table; silent resync.

**Q: What is the VersionMismatch notice, and what kind is it?**
Decision: A new Notice variant, VersionMismatch { field }, in pg-model, lifecycle Transient (toast and log, never kept; notice.rs:594-598), with the next unused numeric code after the highest in use (notice.rs:1297-1331; RETIRED at 769). Words in crates/client/locales/en/text.ftl beside notice-update-available (342-343) and the client text module. Both browser and desktop post it. It is not written to the catalog inbox. The static update-failed page is plain HTML in index.html. Each VersionMismatch is logged as a diagnostic with both versions.
Why: Feedback on what the user just did is the Transient lifecycle. pg-model compiles for wasm (tools/lint.py:21). The inbox is catalog state about the library, and a per-device conflict is not a library fact. UpdateAvailable (notice.rs:581-584) is reserved for version checks and must not be reused.
Rejected: a kept State notice (persists per-device conflicts in the shared catalog); silent adoption; automatic retry (overwrites newer values); a modal dialog.

**Q: What checks and credentials apply to the data and writes, and what is the CSRF answer?**
Decision: POST /api/auth/login sets an HttpOnly cookie with SameSite=Lax (WEB_CLIENT.md:155), bound to a device record the desktop can revoke. Origin is checked on every POST (mutations, login and other state-changing requests), not only on a socket. A cross-site page cannot make an accepted POST with the owner's cookie whatever SameSite does. Reads are GETs. The Host allowlist (decision 2) blocks DNS rebinding for reads. No token or cookie in a URL, log line or the handshake. Revoke and expiry wake held polls and each request re-checks the device. The handshake exposes only build and bundle identifiers, already visible in the public login bundle.
Why: With no socket, cross-site hijacking becomes a POST case, which the POST Origin check covers. This answers the open CSRF question (WEB_CLIENT.md:592) without depending on SameSite behaviour.
Rejected: a CSRF token (a second secret where Origin covers the case); SameSite alone (untested); token in URL.

**Q: What happens on a graceful stop and a restart?**
Decision: A graceful stop stops accepting connections, answers every held poll with the current vector at once, lets in-flight commits finish, and exits. The client's next poll gets a network error, backs off, reconnects to the new process, sees a new epoch and reopens its listings. No close codes. Restart costs one reopen per open listing per client per release (unmeasured).
Why: Without a socket, a stop costs only the next request; answering held polls first means no client waits out a timeout.
Rejected: close codes on a socket; killing the process (an in-flight set becomes unknown).

**Q: Which crate holds the wire types, the build script and the state machine?**
Decision: A new crate pg-wire (crates/wire): depends on pg-model and serde only, is wasm32-clean and added to the wasm32 check (tools/lint.py:21). It holds the build script for build_id, the bundle_id type, the change vector, frozen envelopes and their golden fixtures, error codes, and the pure client state machine. Desktop and browser (the same code compiled to wasm) both use it, each with its own transport. crates/web (pg-web) depends on pg-wire and pg-app and holds the axum server.
Why: The controller's index-based Event is recorded under PG_RECORD with REPLAY_FORMAT_VERSION 3 (replay.rs:11-13), so putting wire types in pg-client would couple them to the replay format. pg-web depends on tokio and axum and cannot be wasm32-clean.

**Q: How is build_id computed in practice, and what about the QML dev loop?**
Decision: The pg-wire build script hashes every workspace crate's src and Cargo.toml, Cargo.lock and the workspace manifest, declaring each with rerun-if-changed. QML is excluded from build_id so the debug live reload does not restart the server. The browser bundle's QML is covered by bundle_id. In the web dev loop a bundle rebuild needs a server restart to be served. Whether the target triple and feature set belong in the digest is open.
Why: Keeps desktop live reload from restarting the background server on every QML save, and keeps the browser's check exact for its own bytes.

**Q: How is each part tested?**
Decision: (a) Golden fixtures for frozen envelopes. (b) A model-based property test: generated interleavings of commits, scope bumps, dropped answers, reconnects, restarts (epoch changes), refetches and late answers, checking convergence after quiescence and that no version lower than one shown is ever shown (`proptest` is a workspace dependency). (c) A deterministic race test of refetch against bump, with a test-only hook (pattern: platform testing/seam.rs:1-4). (d) Conformance cases: an equal vector makes no request; one scope moved re-reads only that scope; an epoch change reopens every listing; an older fetch is discarded; a refetch failure adopts nothing; a hold timeout re-polls at once; a 401 shows login; a build or bundle mismatch on a read and on a set gives 409 with nothing applied; a placeholder becomes an image on the thumbs scope; a pending record reconciles to applied, not applied, or ambiguous. (e) A browser test of the stale-entry guard and hop limit.
Why: These failures are orderings that only generated interleavings find.

Open:

- Out-of-band writer: a second process bypassing the single-writer lock writes without a bump; no reliable detector exists (inbox.rs:8 uses data_version only to re-read).
- A server defect that commits without a bump is caught only as a VersionMismatch diagnostic; clients stay stale on unread records until reload.
- Reload livelock under constant churn: progress is made but the rate is bounded only by the 250 ms floor and coalescing; cost unmeasured.
- Mutation residuals: if another device set the same value after our write, the item is past expected+1 and the outcome is ambiguous, so the notice may say a write did not apply when it applied; if IndexedDB is cleared, no reconciliation.
- Scan-written rows must bump item versions for the items scope and viewport revalidation to be correct; not checked in the catalog writers.
- Membership evaluation per commit for at most 4 listings per device must match each listing's query and sort, including delete and move; cost unmeasured.
- The TLS library decides HTTP/2 for the long-poll. native-tls on Windows has server ALPN behind alpn-accept (schannel.rs:334-366), not enabled here, with a chain FIXME (schannel.rs:360); the conclusion is that rustls is not forced by ALPN alone.
- The desktop's loopback poll needs a blocking HTTP client that can hold for the hold timeout; ureq is the only HTTP client (the `ureq` package in Cargo.lock) but its suitability is unverified.
- Qt WASM: the transfer timeout (qnetworkreplywasmimpl.cpp:276) and whole-body loading (:259) were read in source, not run; behaviour of a 25 s held fetch under Android Chrome with the screen off, and the order of visibilitychange, pageshow and online on resume, untested.
- Bundle rename: whether renaming the Qt emitter's fixed loader and wasm names (and the references inside the loader) gives a working pair is unchecked.
- build_id input set: exact file list, cargo's rerun-if-changed for directory trees, and whether target and features belong in the digest; a missed input under-invalidates.
- Whether Android Chrome honours no-cache with an ETag on index.html under the LAN HTTPS setup: reasoned, not tested.
- Whether Origin is sent on same-origin POSTs from Android Chrome, and that a cross-site GET cannot read the response: standard behaviour, not tested.
- All numbers are proposals: hold timeout 25 s; watchdog margin 10 s; the held-poll cap of `WEB_CLIENT.md` (Limits); 250 ms batching and refetch floor; backoff 500 ms, factor 2, cap 30 s; 3 s wait for in-flight sets; 1 s progress coalescing; hop limit 3; the held-poll cap is the one in `WEB_CLIENT.md` (Limits).
- Restart cost: every release reopens every listing per client; unmeasured. Snapshot grace (WEB_CLIENT.md:221-222) decides how often `snapshot-expired` forces a reopen without restart.
- Frozen envelope discipline relies on people; golden fixtures catch encoding changes, not meaning changes.
- Desktop replacement: the single-instance lock that releases an old background server (WEB_CLIENT.md:266-270) has no code here; side-by-side installs of two desktop builds are not covered by the once-per-start rule.
- LAN address change (WEB_CLIENT.md:593): a client backs off forever with no visible state; a cannot-reach-server state after N attempts is still needed.
- Yandex folder changes must bump the tree and items scopes through the same commit path; not checked.
- Video stays open and is not designed.
- Dev loop: a bundle rebuild needs a server restart; whether the bundle can be re-read without one is undecided.

Measure first:

- Refetch round time for a viewport after a bump, and a real scan's bump rate per scope (to decide whether the reload livelock occurs).
- Cost of per-commit membership evaluation for open listings.
- Cold start of a held-in-memory bundle on a phone; resident cost of holding the 27.7 MB wasm.
- Time to restart and reopen all listings for each connected client.
- Held-poll behaviour of Qt WASM under Android Chrome with the screen off.

Validation notes:

- Corrections: the LAN address change item is at WEB_CLIENT.md:593 (not 458, which is the HTTPS and private-CA text). The native-tls claim at 383-385 is contradicted for version 0.2.18 by schannel.rs:334-366; the choice of rustls is therefore not forced by ALPN alone. notice.rs UpdateAvailable is at 581-584; SessionNotRead's code 46 is at 1333, outside the cited range. Lifecycle Transient 594-598, Notice::Unknown 586-589, RETIRED 769, golden test 1379-1392 and text.ftl 342-343 are correct.
- Verified: Event and Command both derive Serialize and Deserialize (client controller.rs:75-76, 143-144); Event has 21 variants; no sequence number; controller.rs:1-6 time as input; jobs Bus pops the oldest and counts lagged (events.rs:77-89); Change at 127-140; Generation counters (runner.rs:131-141); inbox.rs EVENTS 1024 (line 59) and POLL 30 s (line 62); tools/lint.py:21; Cargo.lock getrandom, blake3, native-tls and ureq package entries (checked at the line numbers then current); the Cargo.toml blake3, proptest and version entries; CARGO_PKG_VERSION used only at desktop lib.rs:144; replay FORMAT 3 at replay.rs:13; clippy.toml bans; qnetworkreplywasmimpl.cpp 259, 271, 276; qwebsocket_wasm_p.cpp has no ping API; live_reload.rs 1-6.
- Contradictions: (1) Step 2 of decision 2 exempts the login POST from the Origin check (decision 19 requires Origin on every POST, login included). Fix the middleware order. (2) Decision 19 does not address a cross-site GET that enqueues work: the thumbs design needs SameSite=Lax plus a Sec-Fetch-Site 403 on the thumbnail GET, and events needs that check too. (3) Thumbnails: this design says public, immutable, with turns in the path and the render version in a query parameter; the thumbs design says private (never public) with thumbs_version in the path and no turns; these cannot both be the contract. (4) No socket: the thumbs design's want list, ready hint and resume(seq) need a channel this design does not provide; placeholder re-request is the thumbs scope here and a per-cell backoff timer in thumbs. (5) Desktop attach: pg-wire placement conflicts with the cutover's rule that trait types live outside pg-app; amend one. (6) Replay and events: the cutover (C5) assumes resume and gap resync, which this design removes; quarantine assumes replay is open, which this design closes.

---

### Wire surface: endpoints, item record, mutations

**Q: Provenance and graft plan.**
Decision: Kept from design 1: the stream and merge stamp (files.changed, extended to every wire-visible column); request ids and the ops row written in the mutation's transaction; the turn delta with a guard; bulk as_of and expected_count with the job persisted server-side. From design 2: binary records and listing-scoped facets; the request-id idea. From design 3: per-field guard versions rather than one item token; selections as FileId ranges plus add and remove; folder tokens; Yandex URL rules; thumbnail miss semantics. The listing header carries FileIds (design 1), not design 3's 4-byte shape blob. The thumbnail key excludes turns (design 2), not designs 1 and 3.
Why: Each part is taken where cited code supports it and rejected where a verified fact contradicts it; rejections appear in the decisions below.

**Q: Which stamp drives the record stream, resync and merge, and which values guard a write?**
Decision: Two clocks, both meta.seq. files.changed (wire seq) is the stream, resync, ETag and client merge stamp: a client replaces a held record only if the incoming seq is greater. Extended to bump on name, folder_id, kind, analysis width, height, orientation and thumbhash; not stamped by last_scan. Guards use three separate columns on files: rating_seq, mark_seq and turn_seq, each the meta.seq of the last user-visible change to that field, set at insert to the born seq and bumped by the replace path. Writer activity (file_writes, write_done, KeptInCatalog, analysis_turned_back) never moves a guard version.
Why: The file_writes triggers (schema.rs:780-782) and the stamp macro (sql.rs:25-45) fire on every writer step, so files.changed moves during every write-back; right for the stream, wrong for a guard, which would fail the user's own next edit against the app's own write. Verified gaps for the stream: files_changed (schema.rs:719-726) updates only pending, so kind is unstamped; analysis_changed (schema.rs:755-758) omits width, height, orientation and thumbhash; the scanner upsert sets folder_id and name without a stamp (writers.rs:114); take_identity does the same (pair.rs:357). The write_done width-height swap (queue.rs:260-262) is covered only by a coincidence of the same transaction; do not rely on it.
Rejected: one item-wide token (couples rating, mark and turn, causing false conflicts across devices); files.changed as the guard (spurious conflicts from the writer); a content hash of the record (recomputed on every read).

**Q: How is each guard field stamped: triggers or application code?**
Decision: rating_seq: AFTER INSERT, AFTER UPDATE WHEN old.rating IS NOT new.rating, and AFTER DELETE triggers on ratings. ratings has two writers: set_rating (queue.rs:42-43) and the scanner's adoption path (writers.rs:280-281; sync_file_rating at 367-371). The WHEN form already exists (sql.rs:43). mark_seq: the border_marks insert and delete triggers (schema.rs:778-779), with borders.rs:25 changing from INSERT OR REPLACE to INSERT ... ON CONFLICT(file_id) DO NOTHING, so a same-value mark is a no-op with no fire (border_marks.file_id is the PRIMARY KEY, schema.rs:607-610). turn_seq: set in queue_rotation (queue.rs:103-127) in the same transaction; its only non-test caller is library.rs:338, the user's turn path.
Why: SET_RATING is an upsert that always rewrites the row (types.rs:341-343), so an unconditional UPDATE trigger would stamp same-value sets. A REPLACE fires only the insert half of its triggers (schema.rs:741), which is why the mark path must use ON CONFLICT. Application-level stamping would miss the scanner's ratings path.

**Q: How is one mutation checked and applied?**
Decision: Every mutation runs in one IMMEDIATE transaction (transaction.rs:13-14). The server reads the current value and field seq. If the value equals the requested value, the result is unchanged: no bump, but the ops row is still written. Otherwise, if the version equals the request's version, it applies, bumps the seq, writes the ops row and returns the new record. If the version differs and the value differs, it writes nothing, returns 409 version-mismatch with the current record, and writes the ops row with the conflict outcome so a retry gets the same answer.
Why: The check and write share one transaction, so no writer interleaves. The value-equal rule lets a retry of a set that already holds its value succeed rather than conflict with itself.
Rejected: a check before the transaction opens (time-of-check to time-of-use); last writer wins for ratings and marks (silently drops another device's write); idempotency without a request_id (a retry after success reports a conflict unless equal; an unknown-outcome turn cannot be retried).

**Q: Request ids, unknown outcomes and retention.**
Decision: Every mutation carries request_id, a random 128-bit id as a decimal string, scoped to the session until device identity lands, then to the device. In the same IMMEDIATE transaction the server inserts an ops row (scope, request_id, outcome, time). A repeat request_id returns the stored outcome. GET /api/ops/{request_id} returns outcome or unknown. Retention: proposed 24 hours or 10,000 rows per scope, whichever is smaller (unmeasured). After retention, an unknown set is decisive only if the item's version still equals the one sent; an unknown turn is shown to the user rather than resent.
Why: The ops row and the mutation commit or roll back together, so the record and answer cannot disagree after a crash (under the synchronous setting this design assumes; see validation).
Rejected: an in-memory request cache (lost on restart); a separate ops store (a crash can commit one and lose the other); deciding an unknown turn from the version alone (a turn is additive).

**Q: Rating wire rule.**
Decision: The wire value is null or an integer 1 to 5. Zero is rejected at decode with 400 bad-request; null is the only spelling of unrated; the server maps null to DELETE_RATING and a value to SET_RATING (queue.rs:41-44). write_back is read inside the same transaction from the catalog meta table, not from preferences before the call. The response reports write as queued, not_queued (write-back off or the file already holds the value; queue.rs:55-66) or not_possible (the format cannot hold a rating; queue.rs:55-56), plus rating_in_file and affects_listing. A rating on a missing file returns 410 gone.
Why: set_rating takes write_back as a parameter (queue.rs:37), so a toggle can race the app's pre-read (edit.rs:27-31); a single transaction reads one value at commit. The response must not claim the rating reached the file when it only reached the catalog.
Rejected: passing 0 through (the Rust type accepts it and the SQL CHECK rejects it); accepting ratings on missing files (queued writes are dropped later, queue.rs:294-301).

**Q: Border mark wire rule.**
Decision: The wire value is boolean marked, with version = mark_seq and request_id. Order: if the value already equals the request, return unchanged. Otherwise marking requires the borders to be Found, using the same derivation as entry_from_row (grid.rs:57 and 74-79); a request failing it gets 422 not-applicable. Unmarking requires the mark to exist. The check runs inside the transaction. The desktop guard at edits.rs:98 stays as a UI guard only.
Why: set_border_mark checks nothing (borders.rs:22-31): a mark on a file whose borders are not found is stored and invisible, reappearing when analysis later finds borders. The value check first makes a retried mark succeed.
Rejected: trusting the client's cached borders state; keeping a relative ToggleBorderMark on the wire (a toggle from a stale display inverts intent).

**Q: Turn wire rule and the guard for rotation.**
Decision: POST /api/items/{file}/turns with {delta: 0 to 3, version: turn_seq, request_id}. The server applies queue_rotation's semantics (queue.rs:103-127, additive mod 4) in the same transaction and bumps turn_seq. delta 0 is unchanged and writes nothing. Values outside 0 to 3 are 400. The delta is never converted to an absolute target, and no turns_asked column is added.
Why: Turns are additive (queue.rs:112), so a delta is exact for any writer interleaving. The writer changes rotation and pending in ways that are not invariant (write_done subtracts written turns, queue.rs:246; KeptInCatalog adds to files.rotation, queue.rs:239 and 357-358; analysis_turned_back, queue.rs:349). turn_seq moves only on user turns, so writer activity causes no conflict. A stale user turn is a conflict.
Rejected: an absolute target from rotation plus pending (not invariant); a turn with no guard (lands on a photo the user did not see); a turns_asked counter (same guard, an extra column and write).

**Q: Key repeat and the one-in-flight rule.**
Decision: A client coalescer per item and per field. At most one request is in flight per item. While one is in flight, rating changes keep only the last value and turn deltas sum mod 4 (a zero sum sends nothing). Each request carries the turn_seq or rating_seq from the last ack and its own request_id. On 409 the client drops that item's queued intents for that field, shows a notice naming them, and re-reads from the 409 body (which carries the record); no silent retry. The coalescer flushes on ack, visibilitychange and pagehide. A 100 to 200 ms batching window is a proposal.
Why: keys.rs:103-141 maps R and digits one command per key event and no repeat filter exists in crates/client/src. Coalescing bounds round trips per burst and keeps the version chain linear. Dropping relative intent on conflict is conservative.
Rejected: one request per key press (N round trips); parallel requests for one item (the second always mismatches); re-sending dropped turn deltas (applies a turn to a photo another device changed).

**Q: Per-item endpoints.**
Decision: PUT /api/items/{file}/rating {value: null or 1..5, version, request_id}; PUT /api/items/{file}/mark {marked, version, request_id}; POST /api/items/{file}/turns {delta, version, request_id}. Each response is {applied or unchanged, record (the GridRecord with new seqs), write?}. Not routed: DELETE /api/items/{file} and POST /api/items/{file}/move (quarantine is dropped, `WEB_CLIENT_TECH_DECISIONS.md:200`).
Why: Each field's rule is visible at its route, and wrong rules are rejected at the boundary. The record in the response removes a refetch after each edit.
Rejected: a generic PATCH with several fields (atomicity across fields is not needed); a generic set with a field name (lets a turn be sent as a set).

**Q: Bulk actions: one endpoint, a job, and as_of.**
Decision: POST /api/selections/apply {listing, generation, selection, change, as_of, expected_count, request_id} returns 202 with a job id. change is rating(value), mark(marked), turn(delta) or save_crops(folder token). At creation the server materialises the selection to FileIds and stores them in the job row with a cursor. Chunks of 500 (proposal) each run in one IMMEDIATE transaction that applies the change, advances the cursor and writes the ops rows, so a crash neither repeats nor skips a chunk. The job runs on the server without the phone. Per member, the guard is the member's field seq at most as_of; a member above is skipped and counted as a conflict, with a capped sample of 100 (proposal). expected_count is recounted in the transaction; a mismatch returns 409 selection-changed with the new count. Progress on the event stream and GET /api/jobs/{job}. One running job per device, two per server (proposals).
Why: as_of is exact because field seqs and the client's sync cursor share meta.seq (sql.rs:17-23). The job row holds the cursor, so work survives a sleeping phone and a restart. A bulk turn uses the same turn_seq guard as a single turn.
Rejected: last writer wins across the selection (overwrites another device); one transaction for all members (holds the write lock across the run; transaction.rs has a hold timer); client-driven chunks with resend (stops when a phone sleeps); per-item versions in the request (189k entries).

**Q: Save-crops as a bulk change.**
Decision: save_crops is a change in the bulk endpoint, carrying the folder token, expected_count and request_id. The server selects members whose borders are Found using the desktop's predicate (edits.rs:120-121 filters on Borders::Found, excluding Marked), inside the job. There is no per-member seq guard; the count is the guard.
Why: Today the desktop builds the id list from the loaded grid (edits.rs:120-124), which is the select-all-as-a-list problem. A crop reads a file rather than writing one, and borders have no version.
Rejected: sending an id list; a fresh query for the predicate (may differ from the snapshot on screen); a seq guard on borders.

**Q: Selection wire format.**
Decision: Selection = {listing, generation, base: none or all, ranges: [[FileId, FileId]] inclusive in snapshot order, add: [FileId], remove: [FileId]}. Members are (base union ranges union add) minus remove. Every FileId in add and range endpoints must be a snapshot member, else 400 not-member. The client sends the shorter form: base none with ranges and add, or base all with remove. Total entries across ranges, add and remove are capped at 4096, else 413 selection-too-large. An expired snapshot returns 410 snapshot-expired; the client does not re-run the query.
Why: Select-all is base all with an empty remove, about 40 bytes; the selection size is bounded by the smaller of selected and unselected counts. Ranges are FileId pairs, so no index crosses the boundary (a decided rule).
Rejected: index ranges over snapshot positions (indices cross the boundary); a FileId list for select-all (about 1.3 MB as JSON, estimate); re-running the query on expiry (a scan between select and act changes the confirmed set).

**Q: Listing header: ids and geometry.**
Decision: GET /api/listings/{id}/header?gen= returns binary, per index: file_id u64 little-endian, shown w u32, shown h u32: 16 bytes per item, tied to listing id and generation. The client maps FileId to index from it. Layout computes aspect as f64 w over h from the integers (Entry::aspect, model grid.rs:79-82; clamp at client layout.rs:16-18). The header is frozen at open. After an edit the new shown w and h arrive in the record and geometry updates by file id. Served with brotli or gzip by Accept-Encoding.
Why: Exact integers give bit-identical aspects to the desktop, so row breaks match. The FileId column maps events to an index.
Rejected: quantised 2 to 4 byte ratios (not bit-identical, so row breaks can differ); a 4-byte shape blob without ids (a second fetch and a per-event position request); thumbhash in the header (25 bytes per item for data only visible cells show).

**Q: Listing snapshot and records.**
Decision: POST /api/listings returns {listing (random 128-bit token), generation, count, anchor_index?}. The server stores the frozen header (16 bytes per item) per listing. Records are re-read per range from the catalog by primary key; a missing file in a range returns gone for that entry. Cap proposed at 4 snapshots per device. A generation mismatch returns 409 generation-mismatch; an expired snapshot returns 410 snapshot-expired. An anchor FileId on open returns its index, or the next item if it no longer matches.
Why: 189,518 times 16 bytes is about 3.03 MB per snapshot, about 12.1 MB for four (arithmetic). The 12-bytes-per-item figure in design 2 omitted the shape. Records re-read by key stay current; the frozen header keeps membership and order fixed.
Rejected: holding full records server-side (about 42 MB per device, from the designs); sequential listing ids (guessable and reusable across restarts).
Note: the snapshot design replaces this model with a client replica (see validation).

**Q: Facets.**
Decision: Listing-scoped only: POST /api/listings/{id}/facets over the snapshot's membership. No live POST /api/facets.
Why: The filter bar must describe the list on screen; a live count would disagree with the frozen list. Snapshot facet cost is unmeasured.
Rejected: live facets (catalog facets.rs:39 reads the current catalog).

**Q: The grid record and the viewer record.**
Decision: GridRecord is binary in a listing range response: file_id u64; mtime u64 (FILETIME, schema.rs:516); shown w u32 and shown h u32 (after pending turns); rating u8 (0 none); borders u8 (unknown, none, found, marked); ext u8; kind and animated flags u8; orientation u8; rotation u8; pending u8; placeholder_turns u8; duration_ms u32 (video is not designed here); thumbhash length u8 and up to 25 bytes; rating_seq, turn_seq, mark_seq and changed as u64. Cut sides are an optional 16-byte column requested with cols=cut when the overlay is on. The record ships no derived strings; badge text, border box and image key are derived in WASM. About 94 bytes without cut sides and 110 with, so about 17.8 MB for 189,518 items as an upper bound. ViewerRecord is JSON via GET /api/items/{file}: path, size, kind, format, rating_in_file, border sides, write status (FileState), error, can_rate, and versions and mtime as decimal strings; read in one transaction with the grid fields.
Why: Grid and viewer have different needs; a per-cell record would carry paths and border sides a range never shows. One transaction keeps versions consistent with fields (reader.rs:349-354 reads them in two calls today). Raw fields once remove a second path for derived values (desktop gallery/model.rs:37-102).
Rejected: one record for both (paths in every range); versions inside Entry (breaks the 56-byte assert, grid.rs:41); server-derived badge text (duplicates client logic).

**Q: Encodings and 64-bit values.**
Decision: Listing data (header, records, shape) is binary and decoded in the same WASM module that runs layout. JSON carries everything else. Any 64-bit value that can exceed 2^53 is a decimal string in JSON: mtime, versions, request ids and Yandex result ids. Thumbnail keys are built in WASM from the binary mtime.
Why: FILETIME in 2026 is about 1.3e17 ticks, above 2^53 (about 9.0e15), where doubles space by 16 ticks; the thumbnail key compares mtime exactly (images.rs:187), so a rounded mtime is a silent cache error.
Rejected: JSON for records (escaping, base64 overhead, precision trap); relying on BigInt parsing in every consumer.

**Q: Thumbhash and the placeholder.**
Decision: The thumbhash goes inline in the grid record, length-prefixed, up to 25 bytes (THUMBHASH_MAX, analysis.rs:138). The client decodes it in WASM. Placeholder turns = ((files.rotation minus analysis rotation) plus pending) mod 4, computed on the server. A null hash (not imported) shows a neutral cell. No per-cell placeholder request.
Why: Verified: Catalog::placeholder computes (f.rotation - coalesce(a.rotation, f.rotation) + 4) % 4 and adds pending (state.rs:85, 96), so design 2's claim that the placeholder ignores pending is false. Visible cells travel with their range, so the placeholder costs at most 26 bytes per item with no round trip.
Superseded 2026-11-17 (commit 5fcff8bb): the hash is the `placeholders` row's (`placeholders.hash`), keyed to the file's identity, and the turns come from `placeholders.rotation` (`(files.rotation - coalesce(placeholders.rotation, files.rotation) + 4) % 4`, state.rs), with the pending turns added as before. A null hash means no placeholder has been made yet (the Placeholders pass makes it after the thumbnail), not "not imported". Changing the placeholder stamps `files.changed` (never `files.seq`), so a client that keeps its records fetches the new hash. The 25-byte bound and the WASM decision are unchanged.
Rejected: per-cell placeholder URLs; thumbhash in the header (about 4.7 MB for 189k); client recomputation of placeholder turns (needs analysis rotation the record does not carry).

**Q: Thumbnail URL, what it names, and the cache rules.**
Decision: GET /api/thumbs/{catalog_id}/t{THUMBS_VERSION}/{file_id}@{mtime}.{rotation}/{height}. No turns component. Exact, versioned key: 200 with Cache-Control private, max-age=604800, immutable. Nearest smaller kept size: 200, no-cache, with X-Thumb naming the smaller height. Miss: 200, no-store, X-Thumb: queued, with the nearest kept smaller size if any, otherwise a 1x1 transparent image (the client shows the thumbhash behind it). An unversioned key returns no-store. The client rotates the decoded image by pending with a CSS transform.
Why: The cache is keyed by source (mtime and rotation); turns are returned separately (images.rs:227, 235) and applied per display (desktop providers.rs:50-59), so cached bytes do not depend on turns; a turns component would refetch identical bytes on every R press. catalog_id stops a rebuilt catalog's renumbered ids matching an old immutable URL. THUMBS_VERSION (render lib.rs:14) stops a renderer change leaving old bytes. no-store on a miss stops an immutable cache pinning the placeholder. A 202 with no body is not used because `<img>` handling of an empty 2xx is unverified.
Rejected: turns in the key; the doc's sketch without version or catalog_id; a 202 for a miss; a placeholder under the final URL with immutable.
Note: the immutable-private choice conflicts with ops and the thumbs design (see validation).

**Q: Miss queue and the request path.**
Decision: The thumbnail request path only reads the cache. A miss queues the original's read and make in a per-device queue (the limit stated in `WEB_CLIENT.md`, Limits), newest visible first. When full, the oldest entry drops and is re-requested when its cell is visible again. The exact-height bytes arrive as an event and the client re-requests.
Why: thumbnail() calls consistent(), which sleeps 20 ms per try up to WRITE_WAIT of 10 s (images.rs:30, 141-161) on the calling thread (images.rs:185); in an async server a blocked request holds the blocking pool, and fast scroll would exhaust it. The decided rule is queue and placeholder.
Rejected: keeping the wait on the request path; an unbounded queue; reading originals on the request path (images.rs:214 today).

**Q: Full view URL and bounds.**
Decision: GET /api/items/{file}/view?fit=WxH or full or region=... Fit sides and the product are bounded by the limits in `WEB_CLIENT.md` (Limits). Above that the server returns 400 and the client asks for a smaller fit. The view is keyed by {mtime}.{rotation}.{pending}, which the server applies; the key includes pending. Unversioned keys return no-store.
Why: full_image applies unwritten turns (images.rs:275-279), so the view's bytes depend on pending, unlike the thumbnail. Fit is parsed with no upper bound (client image.rs:122-124, 138-141), letting the client choose an allocation; a silent clamp would name a size that was not served, so the server rejects instead.
Rejected: silent clamping; no bound; turns out of the view key.

**Q: Item events and the stream.**
Decision: Item invalidations come from a cursor over files, not the in-memory bus: a page of files with changed greater than S (files_changed index, schema.rs:541-543) emits {file_id, changed}; the client fetches records in batches. Membership-affecting changes (a rating or mark that the listing's filter or sort reads, a scan missing a file, a folder removal) trigger a listing reopen. GET /api/listings/{id}/changes?gen=&since= returns a bounded page of member invalidations. The event envelope and resume belong to the sync design; the stream carries meta.seq as its clock.
Why: A stamp is written in the same transaction as the change, so a cursor over stamps cannot lose a change the way a bounded queue can; the bus keeps only the newest events and counts the rest as lagged (jobs events.rs:33-34).
Rejected: the in-memory bus as source (lag loses items); state-carrying events (duplicate what the cursor gives).

**Q: Mutation responses and the echo.**
Decision: Every mutation response carries the item's new record and the request_id. Events carry the request_id of the mutation that caused them, so the originating client suppresses its own echo.
Why: The response removes a refetch; the request_id lets each device tell its own change from another's.

**Q: Thumbnail and listing identity under id reuse and rebuild.**
Decision: Migrate files.id to AUTOINCREMENT, through a table rebuild (SQLite cannot add AUTOINCREMENT to an existing table). Every mutation and listing request carries catalog_id (meta, schema.rs:701-702); a changed catalog_id resets the client. The born seq at insert keeps guards correct even if an id were reused.
Why: files.id is INTEGER PRIMARY KEY with no AUTOINCREMENT (schema.rs:502); pairing deletes rows and displaced rows keep ids (schema.rs:503-504; pair.rs:345, 434), so a reused id under a stale snapshot shows the wrong file.
Rejected: relying on the version alone (a snapshot can show another file under a reused id until it reopens); tombstone rows.

**Q: Replacement keeps the rating and resets the marks.**
Decision: The replace path (pair.rs:426-434) bumps rating_seq, mark_seq and turn_seq. The rating row is kept per the scanner's documented decision 7 (pair.rs:10); the mark is removed by a plain DELETE (pair.rs:429), which fires the trigger. The scanner's owner must confirm keeping the rating; flagged, not changed.
Why: A stale request on the old file must conflict; the mark delete is a plain DELETE, so the REPLACE caveat (schema.rs:741) does not apply.
Rejected: clearing the rating on replace (a scanner decision outside this area).

**Q: Tree and folder identity.**
Decision: GET /api/tree returns a flat pre-order DTO of FolderTree (model tree.rs, explicit accessors) with an ETag that is a hash of the serialised body, not a change seq. A folder inside a listing is named by its Merged parts (rules.rs:49-57, a list of Name values), never by a tree index. A library folder is named by its RootId. Names are JSON strings or {b: hex} for non-UTF-8 bytes (name.rs:230-238).
Why: The tree is small; hashing it avoids a second clock. Tree indexes change when the tree rebuilds.
Rejected: tree indexes as identity; a change seq for the tree.

**Q: Folder browser and write targets.**
Decision: GET /api/fs/roots and GET /api/fs/folders?at=token list folders only, using folders_in (platform dir.rs:21-30) and applying the scanner's refusal rules (RootCheck, dir.rs:36-45) before offering a folder to add. Each folder the phone names as a target (add root, save crops, Yandex save) is an opaque token the server issued for an entry in its own browser listing, bound to the device and with an expiry. The phone never sends a free path.
Why: Save-crops and the Yandex save write into the folder they are given (library.rs:188, yandex.rs:144), so a free path lets a phone name any writable location.
Rejected: raw path strings; a listing that includes files.

**Q: Library, scans and status.**
Decision: GET /api/library/status (library.rs:72-84); PUT /api/library/paused; GET /api/library/roots; POST /api/library/roots {folder token}; DELETE /api/library/roots/{root}; POST /api/library/scans {roots: [RootId] or all}, mapped to paths by the server (library.rs:193 takes paths today); POST /api/library/scans/stop {root}; POST /api/library/rescan; GET /api/library/write-errors?limit. Status changes arrive as events that say only that something changed. A subscriber that sees a lag re-reads status and roots and does not trust events as state.
Why: Change-only events cannot lose state if the client re-reads it. A phone may start scans and imports (decided) but must not name arbitrary paths.
Rejected: treating events as state (lost on lag); polling for progress.

**Q: Yandex.**
Decision: POST /api/yandex/searches {file: FileId}; the server resolves the path (catalogs.rs:48). GET /api/yandex/searches/{sid} answers the status and the full result list so far, without consuming it (`WEB_CLIENT_RECONCILED.md` 1.8). GET /api/yandex/searches/{sid}/results/{id}/image is immutable only for that sid. sid is a token carrying the server epoch and the search generation. POST /api/yandex/saves {sid, result ids, folder token}: the server takes each result's URL from its own map. DELETE /api/yandex/searches/{sid} cancels one search (a new per-search stop, since Yandex::stop at yandex.rs:496 is service-wide). GET /api/yandex/save-folder.
Why: poll() takes found and status on read (yandex.rs:485-489), so with two polling clients each result reaches only one. save_one fetches hit.url as supplied (yandex.rs:336-341), so a save body carrying Hit values would make the server fetch any URL a LAN peer names (SSRF). hit_id is generation shifted 32 bits (yandex.rs:182-184), and the generation starts at 0 in each process, so the sid carries the epoch.
Rejected: consume-once polling; client-supplied Hit objects or URLs; free folder paths; implicit search (sends the photo to a third party, so it must be an explicit action).

**Q: Settings.**
Decision: GET /api/settings returns a whitelist of keys, never the auth store. PUT /api/settings/{key} {value, version} sets one typed key; versioned from migration 10 (`WEB_CLIENT_TECH_DECISIONS.md` B13), last writer wins until then (O17). Scope decides who may set it: write_back is server-wide and owner-only (on by default; in the catalog meta table, see the rating decision); show_borders and the remembered listing per user; row_height, dev_mode and yandex_folder per device. The file write happens first; if it fails the server returns 507 storage-full and the in-memory value is unchanged.
Why: JsonStore::update changes memory and then writes the file, so a failed write silently reverts at restart (store.rs:49-56). Key-by-key writes stop one device's whole-object save overwriting another's change to a different key (Prefs is one struct, prefs.rs:22-44).
Rejected: whole-object PUT; a versioned settings object; keeping the change in memory on failure.

**Q: Saved views and the remembered listing.**
Decision: GET /api/views; GET, PUT and DELETE /api/views/{name}. PUT requires If-Match with the ETag of the view's content, a hash of folder, recursive, query and sort in canonical form; a differing content returns 412. A create uses If-None-Match: *. The remembered listing is PUT /api/session/listing with a `version` from migration 10 (`WEB_CLIENT_TECH_DECISIONS.md` B13); until then it is last writer wins (O17). Its scope is per user (`WEB_CLIENT.md`, State model).
Why: Two devices saving one bookmark name currently overwrite each other (views.rs:51; browse/views.rs:26-33). A content hash gives the guard with no schema change; the ABA case is harmless because the stored content is what the second client saw.
Rejected: a rev column (a migration for the same guard); last writer wins for views (silent loss); one session row for every client (schema.rs:149-151 CHECK id = 1).

**Q: Inbox.**
Decision: GET /api/inbox returns the Listing (entries and unread count). Each EntryId is structured: Row (id, uid, rev) or Ledger (kind, seen), with rev and seen as the version. POST /api/inbox/entries/read and /dismiss take lists of ids and report how many were not applied. POST /api/inbox/retries/{pass}. GET /api/inbox/ledgers/{kind}?limit=&seen_to=. PUT /api/inbox/viewing sets the viewing flag per session.
Why: The inbox already versions entries by revision (catalog inbox/id.rs:8-22), and mark_read updates only where id, uid and rev match, so the wire follows it. The viewing flag is process-wide today (inbox.rs:431), so one phone would change what the desktop is told.
Rejected: string ids; server-rendered notice text (duplicates the client text module); shipping the whole listing on each change.

**Q: Auth and transport as they apply to this surface.**
Decision: The loopback token is accepted only on the loopback listener and the session cookie only on the LAN listener. Host and Origin allowlists apply to every request and to the WebSocket. Unsafe methods (POST, PUT, PATCH, DELETE) require Content-Type application/json and a custom header, so a cross-origin form cannot send them. No secret appears in a URL. Body caps are the ones in the Limits list in WEB_CLIENT.md (64 KiB for mutations, 128 KiB for selections). Per-device rate limits on mutations and bulk creation; per-device caps on snapshots and the miss queue. Event streams close when the device is revoked. reveal_folder is not on the wire.
Why: The threats for a logged-in device are CSRF, SSRF, path abuse and resource exhaustion; each has a control. Desktop-only operations (reader.rs:313) would open a window on the server's desktop.
Rejected: a token in the query string; the loopback token on the LAN listener (a leaked desktop token would work from the phone's network); SameSite as the only defence.

**Q: Error model.**
Decision: JSON bodies `{"error": <kebab-case code>}`, plus the extra fields named below, in one error enum (WEB_CLIENT_TECH_DECISIONS.md B10). 400 bad-request (rating 0, delta out of range, unknown field, strict query parse failure). 401 unauthenticated. 403 forbidden-scope. 404 not-found. 409 version-mismatch (with current record and version), generation-mismatch, selection-changed (with new count). 410 gone (item removed or missing), snapshot-expired. 412 precondition-failed (If-Match not met). 413 selection-too-large. 422 not-applicable (mark rule). 429 rate-limited with Retry-After. 503 busy with Retry-After (SQLITE_BUSY after the busy timeout). 507 storage-full. A thumbnail miss is 200 with no-store.
Why: Each code says what to do next: refetch on 409, resync on 410, back off on 429 and 503, notify on 507. Storage errors are not 500, since the client cannot tell retry from failure then.

**Q: Query on the wire.**
Decision: The wire takes a strict query DTO separate from the stored form. Unknown or unparseable fields are 400. The modified-date range is in this computer's time zone (query.rs:236-238), so the phone sends day bounds as UTC instants with its own offset, and the server applies the offset it was sent.
Why: The stored Query is read leniently (lenient_day, query.rs:211) and drops an unparseable day silently, which is right for saved data and wrong for input. Server-zone bounds would give a phone the desktop's day boundaries.

**Q: Deletes and moves.**
Decision: Deletes and moves are operations on a FileId with a request_id, not field sets. A delete is a Recycle Bin send and is in version 1 (quarantine is dropped, `WEB_CLIENT_TECH_DECISIONS.md:200`); whether a move is in version 1 is open (O40 in `WEB_CLIENT_RECONCILED.md`). They are not guarded by field seqs; the changed value is shown in the confirmation and is not a guard.
Why: A delete concerns the file, not one field; guarding it with changed would refuse deletes because the writer stamps every step (schema.rs:780-782).

**Q: Desktop cutover and the parallel array.**
Decision: Entry stays at 56 bytes (grid.rs:41 assert). The desktop keeps rating_seq, turn_seq and mark_seq in a parallel array in its client data layer, filled from the same records: 24 bytes per item, about 4.5 MB at 189,518 items (arithmetic). After cutover every desktop write (rotate, edits.rs:20-48; set_rating, edits.rs:75-92; set_border_marked, edits.rs:95-113, whose guard at :98 stays as a UI check; save_cropped_copies_picked, edits.rs:115-127, which becomes the crop change) carries the same version and request_id as the phone's.
Why: Desktop and phone write the same catalog; an unguarded desktop write would undo a phone write with no conflict. Growing Entry to 80 bytes adds about 43 percent per item (arithmetic).
Rejected: growing Entry; dropping versions on the desktop.

**Q: Crash, power loss and full disk.**
Decision: A mutation is atomic with its ops row. A crash before commit applies nothing and the client retries with the same request_id; after commit, the retry returns the stored outcome. A bulk job's cursor advances in the same transaction as each chunk. Snapshots live in memory, so a restart gives snapshot-expired and the client reopens. Under synchronous NORMAL a power loss can roll back the last commits, and the mutation and its ops row roll back together. A catalog write hitting SQLITE_FULL rolls back and returns 507 with no ops row, so the retry is safe.
Why: The commit and ops row are one transaction. This is reasoned from SQLite's documented WAL behaviour and was not tested.
Rejected: designs depending on synchronous FULL (a cost not measured here); in-memory version counters (reset on restart).

Open:

- Nothing built, run or measured. Byte sizes are arithmetic on field widths. Proposals and unmeasured: the miss queue, selection cap, body caps and view bound are the limits in `WEB_CLIENT.md` (Limits); chunk 500; conflict sample 100; four snapshots per device; ops retention 24 hours and 10,000 rows; batching window 100 to 200 ms.
- Stamp completeness is asserted, not proven; a test should enumerate every wire-visible column against the trigger set. Analysis width, height, orientation and thumbhash are not in analysis_changed (schema.rs:755-758). The write_done swap (queue.rs:260-262) is stamped only by the same transaction's file_writes update.
- Whether scanner name and folder_id changes (writers.rs:114, pair.rs:357) always move a stamp in the cases the stream needs; a same-size, same-mtime rebind through take_identity was not traced.
- Whether SQLite fires an AFTER UPDATE trigger on an upsert (INSERT ... ON CONFLICT DO UPDATE) with a WHEN comparing old and new values; expected, untested; the first implementation test should check it.
- Whether analysis rows are updated in place or always replaced; if in place for width and height, the stamp set must include them.
- Whether a rating on a missing file (missing = 1) is refused by any layer today; the desktop path was not traced.
- Whether orientation and thumbhash can change without an analysis version bump; not traced for every still format.
- Thumbnail identity by mtime and size cannot tell apart a file replaced with the same mtime and size (tools that keep mtime); the immutable cache then serves old bytes; a content hash would fix it, undecided.
- Browser behaviour on Android Chrome untested: no-store on a 200 image with a 1x1 placeholder; CSS rotation of a decoded thumbnail sized to shown dimensions; Origin and preflight for the custom header; the same-site rule for a LAN address.
- The WASM thumbhash decoder: whether a crate builds for wasm32 in pg-client and its licence were not checked; without one, a JavaScript decoder is needed, which design 2 rejects.
- The 25-byte bound comes from THUMBHASH_MAX (analysis.rs:138) and the vendor README; not checked against real encoder output.
- The writer's cost for a bulk turn or rating over 189k items in chunked transactions is not measured; the write-lock hold must come from a real run.
- Snapshot facets need a membership structure; its cost and the include and exclude checks are unmeasured.
- Device identity is open; request_id scope, the per-device snapshot cap and per-device miss queue depend on it. Until it lands, request ids are session-scoped.
- The event envelope, resume, the handshake epoch and the sync cursor's clock belong to the sync design; this design assumes meta.seq as the event clock, which is not decided.
- The Yandex generation starts at 0 in each process (yandex.rs:203-205; default not read); whether a generation can repeat across a restart within one process was not traced.
- Views' If-Match content hash needs a specified canonical form of folder, recursive, query and sort.
- The real_size scope (device or user) is open and is not on the settings routes.
- Auth details (password storage, lockout, device record, owner scope) are outside this area; only scope tags and listener binding are given.
- Quarantine is dropped (`WEB_CLIENT_TECH_DECISIONS.md:200`); the delete and move rules are provisional on the row-kept assumption.
- The rating-on-replace decision (pair.rs:10) is flagged for the scanner's owner.
- The reveal_folder replacement on the phone is not designed; it is desktop-only and off the wire.
- Video is not designed; duration and video kind appear as placeholders with no video routes.

Measure first:

- Writer time for a bulk chunk over 189,518 items at 500 per transaction; the write-lock hold under the existing timer.
- Snapshot facet cost; include and exclude membership cost (bitmap or sorted ids).
- Byte sizes after brotli or gzip; JSON versus binary parse cost on Android Chrome.
- Cost of the attached ledger transaction per turn against a plain write; ledger growth under scan-rate volume.
- Desktop listing memory per entry (56 bytes, `crates/model/src/grid.rs:41`).

Validation notes:

- Corrections: FileState is at crates/catalog/src/browse/read/state.rs:13 (fields 13-24); lines 29-37 are FileSource, not FileState. The f64 division is Entry::aspect in crates/model/src/grid.rs:79-82, not client layout.rs:24-26 (a doc comment); the clamp at client layout.rs:16-18 is correct. The exact mtime compare is images.rs:187 (source_of at 37-39); 177-183 is the nearest lookup. The turn-rule evidence overstates: WEB_CLIENT.md:230-238 says relative operations are computed by the client into absolute values, so the wire's delta departs from the doc and must be recorded as a doc change, not described as the same rule. The catalog_id row is at schema.rs:701-702. THUMBHASH_MAX is at crates/model/src/analysis.rs:138. JsonStore::update is about store.rs:51-57.
- Unverified: mark_read matching id, uid and rev; InboxEvent::Changed carrying the whole listing; the derived-value range at gallery/model.rs:37-102 (only row_value at 37 seen); the Yandex generation default; the miss queue, selection cap and 64 MP figures (the limits in `WEB_CLIENT.md`) and the 500 chunk (proposal).
- Contradictions: (1) The item cursor emits {file_id, changed} with no request_id, but the echo rule needs the request_id; the ops row has no file id, so the cursor cannot recover the request_id. Store the request_id with the changed row, or drop the echo rule. (2) Ops' durability: the crash decision here assumes NORMAL, but ops moves user-mutation commits to synchronous FULL on a dedicated connection; rewrite this crash statement to FULL. (3) Snapshot memory: ops budgets 56 bytes per entry (from WEB_CLIENT.md:214); this design freezes 16 bytes per item (about 3.5 times smaller). (4) The thumbnail GET here enqueues work and is a GET; ops' route rule says no state-changing route answers to GET; either move the miss queue behind a non-GET or allow a GET that enqueues and justify it against CSRF. (5) Miss queue: resolved to the Limits list in WEB_CLIENT.md (256 queued per device, 2,048 globally; the 2,000 figure is withdrawn). Ops has a visible queue on normal-I/O workers and batch misses on dedicated threads; pick one. (6) Body caps: this design sets 64 KiB for mutations and 1 MiB for selections (up to 4,096 entries, about 80 KB of JSON, estimate); ops sets 16 KiB for JSON mutations, which would refuse a legal selection. Resolved: body caps are the Limits list in WEB_CLIENT.md (64 KiB mutations, 128 KiB selections). (7) Thumbnail caching: this design's exact key is private, max-age=604800, immutable on mtime and rotation (not content); ops keeps thumbnails no-cache until versioned by content; this design's own open item says a same-mtime, same-size replacement serves stale bytes; the key does not meet ops' rule. (8) The decided rule that the CLI's writing commands go through the server while it runs is not defined here (no CLI-as-client rule); ops gives the CLI a direct-write mode.

---

### Operations and hardening

**Q: Where does the LAN listener bind, and what counts as the LAN?**
Decision: The listener binds the IPv4 address of one adapter the owner selects, on an owner-chosen TCP port. It never binds 0.0.0.0 and never all adapters. The LAN is that adapter's IPv4 prefix. The accept loop closes any peer outside that prefix before TLS starts. Adapters are enumerated with GetAdaptersAddresses. RFC1918 is not treated as LAN by class.
Why: A bound address lets the kernel refuse traffic to VPN, Hyper-V and WSL adapters before the application sees it; the peer-prefix check is the second gate for routed traffic. Login remains the trust boundary; neither gate is identity.
Rejected: wildcard bind with a peer filter (the port answers on every adapter); a fixed configured address (the server cannot set the network address); any RFC1918 address as LAN (includes VPN and virtual-switch subnets); peer check alone (spoofable on-link).

**Q: Which firewall rule and network profile does LAN access depend on?**
Decision: When the owner enables the LAN, the desktop reads the current network's profile. On Public it creates no rule and shows that LAN access is off until the owner marks the network Private. On Private it creates one inbound rule for this program and port, remote scope LocalSubnet, Private profile only. Rule presence and profile are checked at each enable and each start; a failed check is a notice.
Why: A rule alone does nothing on Public, and reporting that silently would claim reachability the desktop cannot provide. Windows defaulting new networks to Public is believed, not checked.
Rejected: a rule with no profile check; opening the port on all profiles.

**Q: Is IPv6 part of the LAN listener in version 1?**
Decision: No. Version 1 is IPv4 only for the listener, the peer check and certificate SANs. Link-local and ULA are not LAN addresses in version 1. Recorded as open, not rejected for good.
Why: Per-interface IPv6 needs a zone for link-local binds, a second SAN set and a second firewall rule, none testable yet.

**Q: How is the adapter chosen, and what happens when it loses its address?**
Decision: The owner picks the adapter when enabling. The desktop pre-selects the default-route adapter only when that network is Private; otherwise a choice is required. If the adapter loses its IPv4 address the listener stops and reports. It never moves to another adapter on its own.
Why: A VPN that takes the default route must not move the LAN listener onto it.

**Q: What are the connection caps on the LAN listener?**
Decision: Proposal, unmeasured: at most 64 open TCP connections on the LAN listener and 8 per peer IPv4 address. Before login: 16 total and 4 per peer. Over-cap connections are closed at TCP level before TLS, not queued. Loopback is outside these caps. After login one phone needs about 6 HTTP/1.1 connections plus one WebSocket, which fits in 8.
Why: The browser opens up to six connections per host (WEB_CLIENT.md:113-114), so a per-peer pre-login cap of 2 would stall the login page. Per peer IPv4 because version 1 is IPv4 only.
Rejected: no cap; queueing over-cap connections; pausing accept (stalls the legitimate phone); a per-peer cap alone.

**Q: What may an unauthenticated peer reach on the LAN listener?**
Decision: Before login only: GET of the login page, a 204 for the favicon request, GET of the notices file, and POST of the login route. The login page is one HTML document with inline CSS and script and no other subresource. Every other route, including the app bundle, requires the session cookie and returns 401, or a redirect to the login page for a document request. The 27.7 MB wasm is never served before login.
Why: Implements the decided rule that login comes before any request other than login (WEB_CLIENT.md:274-281). A two-route pre-login surface keeps the limits small and testable.
Note: the favicon and notices routes are exceptions to the decided rule as written; see validation.

**Q: What are the pre-authentication and per-phase timeouts?**
Decision: Per-phase timers, no total request timeout. TLS handshake 10 s. Request headers complete 10 s (hyper-util header_read_timeout, cited, not re-read). Before login, a connection with no completed request in 30 s is closed. After login, HTTP keep-alive idle 60 s. WebSocket pings every 30 s and closes after 90 s without a pong. All proposals.
Why: A total request timeout would cut long thumbnail streams; ping and pong detect a phone that slept without closing its socket.
Rejected: one overall request timeout; OS TCP keepalive alone.

**Q: Must an HTTP response body keep making progress?**
Decision: Yes. A response body write that moves no bytes for 30 s cancels that stream (proposal). The WebSocket's outbound queue is bounded at 32 MiB (Design B); a connection over it closes and resumes by sequence number.
Why: No design states this for HTTP bodies; a phone that stops reading otherwise holds a task and buffer for the whole response.
Rejected: unbounded buffering; bounding only the WebSocket. (The sequence-number resume is superseded by the events design, which has no socket; see validation.)

**Q: How are request body and header sizes limited?**
Decision: No route uses a framework default. The default body limit is disabled at the router; each route declares its limit from the limits in `WEB_CLIENT.md` (Limits: login 4 KiB, mutations 64 KiB, selections 128 KiB) and the route table in `WEB_CLIENT_RECONCILED.md` section 1.10; GET and HEAD 0 bytes. Request headers 16 KiB; HTTP/2 max header list 16 KiB; WebSocket inbound message and frame 64 KiB. Each body is read with a cap of limit plus one byte, as crates/yandex/src/fetch.rs:49-52 does. A test walks the route table and fails on any route without a declared limit. Proposals.
Why: The route-table test makes an undeclared route a failing test rather than a review note. Axum and hyper are not in Cargo.lock, so no framework default can be pinned yet.
Rejected: one global limit sized for the largest route; relying on framework defaults.

**Q: What HTTP/2 limits apply to the LAN listener?**
Decision: Proposal: at most 128 concurrent streams per connection (RECONCILED 5.12); a connection that resets more than 100 streams in 10 s closes. The HTTP/2 and hyper versions added must include the rapid-reset fixes (CVE-2023-44487), a gate on adding the dependency. WebSocket stays on HTTP/1.1.
Why: Reset-rate cap (Designs B and C). The browser queues the rest of a thumbnail grid anyway.
Rejected: a stream cap alone (a fast reset loop passes it); HTTP/1.1 only (loses multiplexing for thumbnail batches).

**Q: How are Host, Origin and Sec-Fetch-Site checked?**
Decision: The LAN accepts a request only when Host is the configured stable name or the bound IPv4 address, with the listener port when present; otherwise 421 before any handler. Every unsafe method and the WebSocket upgrade must carry an Origin equal to this server's scheme, host and port. A Sec-Fetch-Site other than same-origin on an unsafe request or upgrade is refused. Rejected Host and Origin values are logged, truncated to 128 characters.
Why: The Host allowlist blocks DNS rebinding. The browser WebSocket API cannot send custom headers, so Origin is the only check on that path (WEB_CLIENT.md:412-413).
Rejected: Origin alone (a rebinding page reads unauthenticated responses as same-origin).

**Q: How is cross-site writing prevented, and what are the cookie attributes?**
Decision: Every unsafe method (POST, PUT, PATCH, DELETE) must carry the custom header `X-Pg-Client` (value browser, desktop or cli), which is not CORS-safelisted, so a cross-origin request needs a preflight this server never answers (WEB_CLIENT_TECH_DECISIONS.md B4). The session cookie is HttpOnly; Secure; SameSite=Lax; Path=/; no Domain, so it binds to the exact host. No state-changing route answers to GET; a route-table test enforces it. SameSite=Strict is not used.
Why: Strict drops the cookie on every navigation begun in another app, forcing a login on each external link. Lax sends it only on cross-site top-level GETs, which are safe once no GET changes state. Origin on the WebSocket covers what the header cannot.
Rejected: SameSite=Strict; SameSite=Lax without the header.

**Q: What are the session lifetime and device record rules?**
Decision: Proposal: a 256-bit random token; the server stores only its BLAKE3 digest in a device record. Max-Age 30 days, slid forward at most once a day on use. Revoking a device deletes its record. Device records are durable across server restart. Storage location (catalog or server file) is the login design's; this design requires durability.
Why: Max-Age is set regardless of unverified browser behaviour for cookies without it. Hashing the stored token means a copy of the data yields no live session.
Rejected: a session cookie without Max-Age; signed stateless tokens (cannot be revoked without a deny list).

**Q: How is the loopback listener protected?**
Decision: Binds 127.0.0.1 only, on its own port. Every request carries the per-start launch token in a custom header, never the URL. The launch file (`launch.json` in the home folder, owner-only, `WEB_CLIENT_TECH_DECISIONS.md` B9) holds the token. Host must be 127.0.0.1:port or localhost:port. No CORS headers. Loopback is outside the LAN caps and limits and is not a login path. The desktop's stop and protocol commands go through it.
Why: The token is a blind-request guard, not a secret against same-user processes. Exempting loopback stops a LAN attacker locking the owner's desktop out.
Rejected: a token in the query string; loopback under the LAN limits.

**Q: How are login attempts admitted and limited?**
Decision: Proposal: for each peer IPv4 address, after 5 failures within 15 minutes, each further attempt waits with exponential backoff from 1 s to 15 minutes. The budget is checked before any hashing. Hashing runs one at a time on a dedicated blocking task, with at most 4 logins queued; a further login gets 429 with Retry-After. No account lockout and no global failure counter that refuses login. Counters are in memory and reset on restart. A failed and a successful login take the same route and return the same response shape. The password is never logged, returned or echoed.
Why: Per-address backoff stops one device guessing; a global counter would let one LAN peer refuse the owner's own login for a window. The queue bound, not a counter, bounds CPU, so the denial lasts only as long as a flood. Reset on restart is accepted; a restart needs the owner or a crash. Hash parameters belong to the login design.
Rejected: a global failure counter; account lockout; account backoff; CAPTCHA (no UI on the phone target).
Note: the auth design persists these counters across restarts; see validation.

**Q: What is logged, and what is never logged?**
Decision: The access log records method, route template (not the expanded path), status, latency and peer IPv4. It never records cookies, Authorization headers, the loopback token, login bodies, query strings or any request body. Auth failures, rate-limit trips, cap hits and rejected Host or Origin values are logged with the peer address to gallery.log. Panic messages must not include request data; the panic hook (desktop panic_log.rs:1-10) is reviewed under the same rule. Rotation uses pg-log's capped rotation (log lib.rs:3); proposal 5 MiB by 3 files, following the notifications plan (cited, not re-read).
Why: Logs sit in the data folder and are shared on fault reports; the route template keeps ids out; the peer address is the owner's only evidence of an attack.
Rejected: default request tracing with headers (writes the cookie on every request); logging secrets for debugging.

**Q: Which TLS library carries the LAN listener, and which crypto provider?**
Decision: rustls for the LAN listener with the ring provider, and rcgen with ring for issuance. ring is chosen because ureq's rustls feature uses ring (ureq's own `rustls` feature), so one provider serves both if the HTTP client moves. The certificate chain and key load from PEM bytes in memory, never from the Windows store. The ring MSVC build and rcgen's licence are checked before adoption (open).
Why: Three native-tls claims were checked in the registry source. (1) native-tls 0.2.18 has a schannel server accept path behind alpn-accept (schannel.rs:342-375), so WEB_CLIENT.md:383-385 is wrong that it has no server ALPN. (2) Its acceptor carries a FIXME that the certificate chain may be missing (schannel.rs:360), which would break a three-level chain. (3) Each identity is imported into a persistent CryptoAPI container named from a process-local counter (schannel.rs:117, 145-149), so two processes can request the same name; effect untested. rustls sends the full chain, takes ALPN directly (rustls-0.23.46 src/server/hs.rs:99, cited) and never touches the store.
Rejected: native-tls for the listener (chain FIXME and container naming); an aws-lc-rs provider (C-backed, unmeasured Windows build, no benefit while ureq uses ring).

**Q: How is the private CA organised?**
Decision: Three levels. Offline root: ECDSA P-256, 10 years, pathLen 1, created at LAN setup. Intermediate: ECDSA P-256, 5 years, pathLen 0, signed by the root at setup and each intermediate renewal, with name constraints: permitted IPv4 is the selected adapter's prefix at signing, permitted DNS is the stable LAN name. Leaf: ECDSA P-256, 90 days, issued by the intermediate with SANs for the stable name and current IPv4. The server sends [leaf, intermediate]. The phone trusts only the root. The server never persists the root private key. After signing the intermediate, the root key is written to an owner-chosen file, read back, its SHA-256 compared, then dropped from memory. Intermediate renewal and a new intermediate after a network change need the owner to supply the root file.
Why: With the root on the server, a compromise of the process most exposed to the LAN would let an attacker trust-anchor every phone. An intermediate limits compromise to what it can sign, and name constraints bound that if verifiers enforce them (open). Rejected: root key on the server; a CA with no name constraints; a single long-lived leaf (stale IP, long exposure); a leaf without the intermediate in the chain.

**Q: How are the CA and leaf private keys stored?**
Decision: The intermediate private key is stored DPAPI-protected (user scope) in the data folder. The leaf private key is in memory only and issued fresh at each start and renewal. DPAPI is mandatory: if unavailable, enabling the LAN fails with a message and no plaintext key is written. Win32_Security_Cryptography would be added to the `windows-sys` features in `[workspace.dependencies]` (feature name believed, not checked). Key files use the write-then-rename pattern (store.rs:68-75).
Why: DPAPI removes a plaintext fallback that would turn a DPAPI failure into silent weakening. A leaf key in memory removes a stored key class. DPAPI does not stop malware running as the owner; accepted, not claimed as protection.
Rejected: plaintext keys in the data folder; keys in the Windows certificate store (native-tls container problem; store writes at each renewal); keys in the catalog (copied into backups).

**Q: How and when is the leaf renewed and reloaded?**
Decision: Proposal: the leaf is checked hourly, and on resume from sleep and on each address change. Reissued when fewer than 30 days remain, or when its SAN set differs from the stable name and current address. Backdated 5 minutes for clock skew. The new certificate is written to a temp file, synced, and renamed (store.rs:68-75), then swapped into a rustls certificate resolver. Open connections keep their certificate until they close. A failed renewal keeps the old leaf until its own expiry, logs, and raises a notice.
Why: A restart to reload would drop every connection. Atomic rename is the repo's own pattern. Keeping open connections on the old certificate avoids dropping a streaming phone.
Rejected: restart to reload; a daily check only (a sleep across the window can miss it, which the resume check covers).

**Q: What happens at expiry, and which notices are raised?**
Decision: At expiry the handshake fails and LAN access stops; no plain-HTTP fallback. Loopback and the catalog keep running. The desktop raises notices (Notice enum, notice.rs:352, extended): at once on a leaf renewal failure; for a leaf not renewed, 14 and 3 days before expiry; for the intermediate, 90 days before expiry (it needs the root file); for the root, 1 year before expiry (the one event that needs the phone to install a new root).
Why: Fail-closed is the consistent choice; falling back to HTTP would send the login in clear and break the secure context Qt's threaded build needs (WEB_CLIENT.md:452-458).
Rejected: HTTP fallback at expiry; silent CA rotation.

**Q: How does the phone get and verify the root certificate?**
Decision: The desktop exports the root certificate (DER or PEM) and shows its SHA-256 fingerprint. The owner moves the file to the phone by a channel the owner chooses and compares the fingerprint at install. The root is never served over the LAN.
Why: A root served over the LAN can be replaced by a LAN attacker, and the phone trusts nothing before the install.
Rejected: serving the root over the LAN HTTP port; mkcert (installs into the Windows root store, changing system trust; Qt's dev server uses it, qtwasmserver.py:55-61); CertCreateSelfSignCertificate (persistent containers).

**Q: How does a DHCP address change reach the certificate, the listener and the phone?**
Decision: The phone addresses the server by the stable name the owner configures. Leaf SANs carry that name and the current IPv4. The desktop watches its adapter's address with NotifyIpInterfaceChange (IP Helper feature believed needed, not in the `windows-sys` features) and polls every 30 s as backstop. On a change it: issues a new leaf with the new SAN set; binds a new socket on the new address (make before break); swaps the certificate; closes the old socket. A cookie is bound to the host the phone used, so a phone that used the address logs in again. The setup screen recommends a DHCP reservation.
Why: Make-before-break removes the rebind window that made a wildcard bind attractive. The stable name in the SAN survives lease changes; an IP-only SAN breaks on each lease change.
Rejected: re-pairing on address change (no pairing exists); a name-only SAN (router DNS may not resolve it; open); binding 0.0.0.0; mDNS as the only discovery (Android .local support unverified).

**Q: How does the server start at logon, and who supervises it?**
Decision: A per-user Task Scheduler task, created with schtasks /Create from an XML definition the desktop writes, without administrator rights (believed, open). The XML sets: action 'gallery serve'; a logon trigger, always on (O5); restart on failure 3 times at 1-minute intervals (proposal); multiple instances IgnoreNew; no execution time limit (PT0S), since a default limit (believed 72 hours) would stop a server; allowed on battery. The desktop starts the server by running the task (schtasks /Run), not by spawning a child. The desktop creates the task, and its logon trigger is always on (O5), so the server runs while the desktop is closed. Task Scheduler is the only supervisor. The desktop shows the last result and offers to run the task again; it does not restart the server itself.
Why: The desktop runs the task, so the server outlives the desktop, and the detached-spawn flags were not verified. The kill-on-close job from spawn_background (process.rs:102-132, KILL_ON_JOB_CLOSE at 115) would kill a child spawned by the desktop. Two supervisors would fight.
Rejected: a Windows service (administrator rights; different account and profile; DPAPI scope wrong); Run key or Startup shortcut (no restart, no history); the desktop spawning the server (killed with the desktop); COM task creation (needs an unlisted windows feature).

**Q: How is a second server prevented, and how do the CLI and a starting server avoid interleaving?**
Decision: The server takes a non-blocking exclusive lock on `<catalog_dir>`/server.lock beside the catalog, through a new try-only function next to the dbfile Lock (Lock::take waits about 5 s, lock.rs:11-13 and 27-38). Held for the process's life; released by the OS on any exit. After taking the lock the server writes its launch file launch.json (port, token, protocol version, build id; B9) atomically. A run file found while the lock can be taken is stale. Exit codes: 0 when another live server holds the lock and answers the health probe; 75 when the lock is held by something that is not a server (a CLI direct write or a rebuild), so the task's restart policy retries and then stops. The CLI direct-write mode takes the same lock exclusively for the whole command, waits about 5 s, then fails with a message.
Why: Keyed on the catalog, the unit that must have one writer. Holding the lock across the CLI write closes the check-then-write race. Exit 0 for every held lock would tell the desktop a server runs when a CLI write holds the catalog.
Rejected: a named mutex (not keyed on the catalog); a PID file (PIDs reused); a fixed TCP port as the lock; the CLI checking for a server and then writing (a server can start between).
Note: this design's exclusive server lock conflicts with the decided rule that ordinary writes share the catalog; see validation.

**Q: How does the desktop attach to a running server, and what happens after a build change?**
Decision: On attach the desktop reads launch.json, checks protocol version and build id, and probes health with the token. If the protocol or build id differs from the desktop's, it sends the authenticated loopback stop command, waits for the lock to free, and runs the task again. It never stops a matching server.
Why: A mismatched server serving the wrong protocol is worse than a brief phone reconnect; each long operation is resumable, so a restart loses at most one batch.
Rejected: leaving a mismatched server running; two servers on one catalog (the lock prevents it).
Note: the events design uses build equality only; see validation.

**Q: How do sleep and power settings interact with long jobs?**
Decision: The server does not stop the PC sleeping by default. While a job runs (scan, import, thumbnail pass) its thread holds system-required execution state through SetThreadExecutionState, released when the job ends. A setting keeps the system awake permanently, default off. The server opts its own process out of execution-speed power throttling only while batch work is queued, and only if measurement shows throttling costs throughput. The windows-sys feature may need adding (believed).
Why: A long job frozen by sleep resumes only when the process runs again. A permanent keep-awake costs power on a machine the owner also uses, so it is the owner's choice. The per-thread lifetime is believed, not verified.
Rejected: doing nothing (a long import freezes at sleep); always awake by default.

**Q: How are I/O priorities set for background reads, and what stays at normal priority?**
Decision: Thread CPU priority stays as it is (priority.rs:12-25). Background I/O uses a per-handle hint, FileIoPriorityHintInfo through SetFileInformationByHandle, on the handles that scan, import, hashing and non-visible thumbnail workers open. The hint is a parameter of the open path (platform file.rs:12-30), and the hashing read (analysis analyze.rs:40) and the thumbnail miss read (render maker.rs:86) must also get it (see validation). Visible-range misses, viewer reads and cache serving keep normal priority. Enabled by default only after measurement shows it changes disk time on the owner's drives. If Windows refuses the hint on a regular handle, the background class has no I/O effect and the design runs on CPU priority alone.
Why: Per handle, only the handles that should yield are lowered, so no thread-level state needs restoring. No batch thread that shares a lock inherits a background mode (the priority-inversion case; not re-read). The current code sets only CPU priority (priority.rs:24).
Rejected: PROCESS_MODE_BACKGROUND for the server (the repo measured 64 files/s against 415 at normal priority, THUMBNAILS.md:78-82, though the mode name there is PROCESS_MODE_BACKGROUND_BEGIN); THREAD_MODE_BACKGROUND_BEGIN on batch threads (sets thread state; unmeasured here); relying on CPU priority to lower disk reads.

**Q: Where do thumbnail misses and batch workers run?**
Decision: A miss is queued and the phone gets a placeholder (decided). The original read at images.rs:214, inside thumbnail() (images.rs:173), moves off the request path to a worker. Batch workers (the non-visible pass) run on dedicated std threads that set their CPU priority at start (as jobs pipeline.rs:131 and 166 do), not on the tokio blocking pool. Visible-range misses go to a separate queue served by normal-I/O workers, so a visible view never waits behind the batch.
Why: Dedicated threads avoid reused-pool-thread state problems; the separate queue keeps interactive misses from waiting behind batch work.
Rejected: reading the original on the request path with a timeout (a timed-out read continues in the background); keeping the on-request read for the desktop only (the shared path keeps the bug).

**Q: How is the frontend bundle named, versioned and checked at start?**
Decision: Bundle files carry content-hashed names (BLAKE3, the crate believed to be a workspace dependency, not checked). A manifest lists each name and hash. The build id is a hash of the manifest, and index.html carries it. At start the server checks every manifest file exists and matches its hash. On failure the LAN listener does not start and the desktop reports the bundle as missing; loopback and the catalog keep running. Bundle files are served only with the session cookie, except the login page.
Why: Designs B and C both fail the whole server start on a missing bundle, which would stop the catalog over a LAN-only asset; the failure is narrowed to the listener. Serving behind login follows the pre-login rule.
Rejected: failing the whole server start; query-string versions (some caches key on path only).

**Q: How is the bundle compressed?**
Decision: At build time each file is written as .br (brotli, highest quality the build time allows, measured) and .gz (gzip level 9) beside the original. The server selects by Accept-Encoding and sends Vary: Accept-Encoding (whether the static-file layer sets Vary is open). Files are streamed from disk, never read into memory. Nothing is compressed per request. wasm is served as application/wasm.
Why: Brotli for size, gzip fallback, following Qt's own server which compresses wasm with brotli (qtwasmserver.py:91-96, cited). Streaming keeps 27.7 MB out of server memory.
Rejected: per-request compression; brotli only.

**Q: Which headers do documents and bundle files carry?**
Decision: Documents carry Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp. Every response carries Cross-Origin-Resource-Policy: same-origin and X-Content-Type-Options: nosniff. index.html and the login page are no-cache with an ETag. Hashed bundle files are private, max-age=31536000, immutable (private because the bundle sits behind login). API JSON is no-store. Thumbnails are immutable only when their URL carries a version that changes with the content; until the thumbnail design decides that, they are no-cache. A Content-Security-Policy (default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none') is a target, not adopted until the Qt loader's needs are checked.
Why: The isolation pair matches Qt's test server (qtwasmserver.py:67-69, cited); require-corp breaks nothing because every subresource is same-origin. Immutable thumbnails on unversioned URLs would serve stale images after an edit.
Rejected: COEP credentialless (untested on the target); Qt's looser cross-origin policy; immutable thumbnails on unversioned URLs.

**Q: How is the Emscripten version pinned and checked?**
Decision: Pin Emscripten 5.0.5 in the Justfile beside the FFmpeg pin. tools/doctor.py (proposed change: it has no emsdk check today) checks the emsdk the build actually uses, found through the EMSDK variable, not a fixed location. In this environment EMSDK names C:/Qt/emsdk, which does not exist, while /c/emsdk/upstream/emscripten/emscripten-version.txt reads 5.0.5. The build fails when the reported version differs. Qt's configure check (QtPublicWasmToolchainHelpers.cmake, cited) is the second guard.
Why: The kit records QT_EMCC_VERSION 5.0.5 (qconfig.h:73). A check on a fixed path would pass on a machine whose EMSDK points elsewhere.
Rejected: the newest Emscripten (Qt's configure fails on a mismatch); no pin.

**Q: How are licence notices produced and served for the bundle?**
Decision: The build generates THIRD_PARTY_NOTICES from the Qt kit's SBOM files (C:/Qt/6.12.0/wasm_multithread/sbom), the Emscripten licence (MIT, /c/emsdk/LICENSE), and Rust licences from cargo-deny. It includes the qtloader.js licence line (LicenseRef-Qt-Commercial OR GPL-3.0-only, qtloader.js:2). Until the owner chooses the Qt option, the notice lists both and says the choice is pending, and the build claims neither. Served at /notices without login (no secret). The repo gets a root LICENSE file, which it lacks today (the owner decides its content).
Why: A generated notice does not drift from the build. The pending-choice rule avoids silently taking the LGPL route. The repo's licence is undeclared, which the owner must settle before distribution.
Rejected: shipping without a notice; choosing LGPL silently; serving the notice behind login.

**Q: What durability do acknowledged writes get?**
Decision: Transactions that record a user mutation (rating, delete or move intent, device record, edit) commit with PRAGMA synchronous = FULL on a dedicated mutation connection, in the server and the CLI's direct-write mode. Batch writes (scan batches, thumbnail writes, rebuild staging) keep NORMAL, since a pass rebuilds them. The shared SETTINGS string (schema.rs:128, verified: synchronous NORMAL) keeps NORMAL as default. synchronous is per connection and cannot change inside a transaction, so the mutation connection is separate. In WAL mode a FULL commit syncs frames written before it (believed from SQLite's WAL semantics; to verify).
Why: An acknowledged mutation then survives power loss, and re-derivable batches do not pay an fsync per commit. Design B's 'CLI stays NORMAL' is rejected because the CLI acknowledges mutations too.
Rejected: FULL everywhere (an fsync per batch, unmeasured); NORMAL everywhere (a power loss can undo a mutation the phone was told succeeded); replaying an event log (not durable in this design).

**Q: How does the server behave when a disk is nearly full?**
Decision: Free space is read with a new free-bytes function, since volume.rs:17 passes null for free bytes and reads only the total. Free space is checked every 60 s and before a large job, on the catalog volume, the thumbnail cache volume, the data folder and the originals' volume for write-back. Background writes stop below max(1 GiB, 2 percent of the volume) free; user mutations stop below 256 MiB free (both proposals). The thumbnail cache cap is the lesser of its configured cap and a free-space value; the default today is total/20 clamped to 1 to 50 GB (thumbcache config.rs:23-35), reading only the total. The migration backup removes its .partial file when its copy fails (rebuild.rs:43-47 leaves it today) and is refused when free space is below the catalog size plus the floor.
Why: Relying on SQLite errors leaves no headroom for WAL growth, and a rollback may need space itself.
Rejected: relying only on SQLite errors; a fixed cap with no free-space check; exiting on a full disk (reads still work).

**Q: How do crashes, logoff and layout changes interact?**
Decision: No design depends on a graceful shutdown. A mutation is acknowledged only after commit. A crash, logoff and shutdown are one case. Layout changes run only at process start, inside Catalog::prepare before any open in that process (core.rs:69, 92), so a live server never needs an exclusive hold. A rebuild while the server runs is a stop, a rebuild and a start, with the stop sent through the authenticated loopback command. Startup removes a leftover .building before any open (rebuild.rs:67-68 does this only at the next rebuild today).
Why: A rebuild is rare, so a stop costs little. Design B's self-release of the shared hold would make same-process lock behaviour load-bearing; startup-only avoids that.
Rejected: a live layout change with the shared hold released (complex for a rare event, and same-process lock behaviour untested); a second process for rebuilds (a second writer).

**Q: What does the desktop do when the server is down?**
Decision: After cutover the desktop never writes the catalog. When the server is down it shows that state, runs the task, and retries. It has no in-process write path.
Why: The decided sole-writer rule. Designs A and B agree.
Rejected: an in-process fallback write path (a second writer).

**Q: How does the server handle a phone that sleeps or drops, and how big can snapshots get?**
Decision: When a client's socket closes, its thumbnail batch is cancelled; decoded thumbnails stay in the cache. The phone reconnects with backoff: 1 s doubling, capped at 30 s, with jitter, then resumes by sequence number (the State model). A server-wide snapshot memory budget of 256 MB (proposal) releases the least recently used snapshots above the per-client cap of four. The snapshot grace length stays open.
Why: A per-client cap alone does not bound total memory across many clients. The 56-byte-per-entry figure is cited from WEB_CLIENT.md:214 and not re-measured.
Rejected: a per-client cap only; continuing a batch for a closed socket; reconnect without backoff (a restart produces a reconnect storm).
Note: the events design has no sequence resume and the snapshot design removes server snapshots; see validation.

**Q: What do the docs need correcting on in this area?**
Decision: (1) WEB_CLIENT.md:383-385 says native-tls on Windows has no server-side ALPN or accept path; schannel.rs:342-375 contradicts that. (2) WEB_CLIENT.md:274 and 458 say a private CA is already configured; no CA tooling or CA file appears in the repo's crates. (3) The decided rule that login comes before any request other than login (WEB_CLIENT.md:274-275) is the basis, with the exceptions listed in the open items.
Why: A design built on a wrong library claim or an unlocated CA carries the error into the build.

Open:

- Phone trust in Chrome on Android: whether a user-installed root is accepted for the stable name and the IP, whether name constraints on the intermediate are enforced, and whether a 90-day leaf is accepted. The decided phone trust test is the gate.
- Whether verifiers enforce name constraints on the intermediate (Chrome Android for user roots, and any desktop verifier). If not, the constraints document intent only and the damage bound does not hold.
- Network change: after the adapter's subnet moves outside the constrained prefix, the server cannot issue a leaf for the new address and needs the root file to sign a new intermediate. Laptop owners would hit this; the policy (notice, then manual re-sign) is a proposal.
- Which Qt licence option the owner takes (LGPL-3.0-only with relink obligations for a statically linked library, or Qt commercial), and whether relinkable objects must be provided. The project's own root LICENSE. Legal review needed; not legal advice.
- Whether serving the bundle to the owner's own phone counts as distribution, and the GPL obligations of the GPL FFmpeg build the server loads (crates/README.md:218-226, PLAN.md:327). Not decided.
- Whether firewall rule creation needs elevation, and whether a new network defaults to Public (both believed, not checked).
- Task Scheduler: per-user task creation from XML without administrator rights; whether restart on failure applies only to non-zero exits and to runs started by Run; whether logoff ends the task; the default execution time limit (believed 72 hours); whether the schtasks XML route needs no extra windows feature.
- windows-sys feature names not in the `windows-sys` features of `[workspace.dependencies]`: Win32_Security_Cryptography (DPAPI), the IP Helper feature (NotifyIpInterfaceChange), Win32_System_Power (SetThreadExecutionState), all believed; whether SetFileInformationByHandle with FileIoPriorityHintInfo needs no new feature.
- Whether FileIoPriorityHintInfo changes read time on a regular file handle, not only on a volume handle (NTFS_MFT_SCAN_PLAN.md:142-143 records a volume handle only). Decision on the hint is conditional on it.
- Whether THREAD_MODE_BACKGROUND_BEGIN lowers disk time for a thread's reads, and whether the 64 files/s result (THUMBNAILS.md:78-82, named PROCESS_MODE_BACKGROUND_BEGIN) applies to thread mode. Not measured in either form.
- Whether SetThreadExecutionState's requirement holds for the job thread's lifetime and keeps the system awake when the thread is the only holder (believed).
- Whether a FULL commit in WAL syncs earlier NORMAL commits, and the durability of synchronous FULL on the owner's disks, including write caches that lie about flushes. Not tested.
- Same-process file-lock semantics (LockFileEx per handle). Decision on the mutation connection avoids depending on it; the cross-process tests (dbfile tests/live.rs:30-68) cover the other case.
- Whether tower-http ServeDir precompressed sets Vary: Accept-Encoding, and whether RequestBodyLimit and hyper-util's header_read_timeout, max_concurrent_streams and keep_alive_timeout exist as cited (registry sources not re-read).
- Whether the locked hyper and h2 versions include the rapid-reset fixes; Cargo.lock has no h2, hyper, axum, tokio or rustls entry, so the gate applies when the server crate is added.
- The Qt loader's CSP needs (eval use; whether a policy can be set without breaking the threaded kit). Not checked.
- Whether the login page can be a single document with no subresource, and how many connections a phone's browser opens to it (the pre-login cap of 4 depends on it).
- The stable LAN name: whether the owner's router resolves DHCP hostnames, and whether Android Chrome resolves .local names. Untested.
- Whether the mutation-connection split is compatible with the catalog's connection model (core.rs opens connections in one place); not traced end to end.
- Argon2id parameters and hash time on the owner's machine (OWASP starting point 19 MiB, 2 iterations, 1 lane, cited by Design B; not measured); the login design owns them.
- Session cookie across a closed Chrome on Android with Max-Age (unverified); device record storage location is the login design's.
- Thumbnail URL versioning for immutable caching: the thumbnail design must decide it; until then thumbnails are no-cache.
- Snapshot grace length and the 256 MB budget are proposals owned by the State model.
- The spike figures (27.7 MB wasm, 9.6 MB gzip, 294 KB JS, cold start) are quoted from WEB_CLIENT.md:474; the spike output is not on disk, so they were not rechecked.
- Whether the desktop's pre-screen on 127.0.0.1 counts as a secure context for Qt's threaded build (browser rules, not run).
- Whether DPAPI protection is what the owner needs, since malware running as the owner can read the same files; stated as accepted.
- The rcgen crate's licence, maintenance and deny.toml compliance; not in Cargo.lock; the ring MSVC build and any cmake or NASM need; not built.
- Whether rustls with the platform verifier accepts the same Yandex chain that native-tls with PlatformVerifier accepts (http.rs:30-38); part of the TLS unification question.
- Whether a desktop's sleep or hibernate stops the task and the server, so the phone gets no response until wake (laptop power behaviour not checked).

Measure first:

- Thumbnail batch throughput at normal priority, with a per-handle low hint, and with thread background mode, on the library drive and the cache drive; interactive miss latency during a batch (decision 28).
- Whether the power-throttling opt-out changes batch throughput (decision 27).
- Argon2id hash time on the owner's machine and login latency under a flood (decision 15).
- Browser connection count to the login page and to the app on Android Chrome (sets the caps in decision 5).
- FULL versus NORMAL commit latency on the owner's disks for user mutations (decision 36).
- Brotli size and time for the 27.7 MB wasm, and the effect of a wasm-opt or debug-strip pass (decision 31); cold start on Android Chrome with gzip and brotli.
- Free-space floors from measured WAL growth during a large import (decision 37).
- Time to rebind and reissue after an address change (decision 23).
- Windows checks: the task (non-admin creation, restart on non-zero, logoff, default time limit) and the firewall rule (elevation, Public default); Android Chrome trust of a user root, name constraints and a 90-day leaf.

Validation notes:

- Corrections: the I/O hint decision says file.rs:12-30 (open_read) is the one place reads open files. That is false. The hashing read opens with File::open at analysis analyze.rs:40 (FileHash::of_reader), and the thumbnail miss read uses std::fs::File::open at render maker.rs:86 (read_file, called from images.rs:214). Only the scan pipeline uses open_read (jobs pipeline.rs:288). The hint must be applied at analyze.rs:40 and maker.rs:86 too, or the hash and miss reads are not covered.
- The login-before-everything rule is at WEB_CLIENT.md:274-275; lines 277-281 hold the safeguards and login bullets. Citation drift only.
- Emscripten pin: tools/doctor.py has no emsdk or emscripten check today (grep found none); present it as a proposed change.
- native-tls: the server accept path (schannel.rs:342-375) exists but is behind the alpn-accept feature, which this repo's build does not enable; the FIXME is verified.
- Decision: the desktop stays the catalog's only writer after cutover. Its start keeps `Catalog::prepare` (through `pg_library::open_catalog`, `app.rs:62`), which can migrate, rebuild or move the catalog aside under the exclusive hold (`core.rs:67-92`). The web server opens the catalog read-only and never calls `prepare`.
- Contradictions: (1) The exclusive server lock on `<catalog>`.server.lock for the server's whole life conflicts with the decided rule that the exclusive hold is for layout changes only; the shared hold kept by the keeper (core.rs:124, app.rs:62) is the mechanism the decided rule relies on. (2) The CLI direct-write mode taking the exclusive lock for the whole command and failing while a server runs is a refusal, which the decided rule rules out (the decided rule routes writes through the API). (3) The CLI direct-write mode is a second writer when no server runs, while the design rejects a second process for rebuilds as a second writer; the same rule applies to both. (4) The pre-login route set includes a favicon and a notices GET, which are exceptions to the decided login-first rule as written; either remove them or amend the rule. (5) The desktop's start prepare (see above) conflicts with the sole-writer rule after cutover. (6) Body caps: resolved to the Limits list in WEB_CLIENT.md (64 KiB for mutations, 128 KiB for selections); the 16 KiB JSON figure in this design is withdrawn. (7) The snapshot memory budget here uses 56 bytes per entry; the wire design freezes 16 bytes per item. (8) Thumbnails: this design says private and no-cache until versioned; the wire design sets an exact versioned key to private immutable. (9) Durability: this design moves user mutations to synchronous FULL; the wire design's crash decision assumes NORMAL. (10) Login lockout state: this design keeps login counters in memory and resets on restart; the auth design persists them across restarts. (11) The reconnect cap and backoff in the snapshot grace text use sequence-number resume, which the events design removes. (12) Attach rule: this design requires build equality; the events design also requires build equality, but this design's protocol check in the attach text names protocol version too; align the two. (13) The desktop's logon task default is off here; the decided rule says the login works while the desktop is closed, so after a reboot with the setting off the phone gets no server until the desktop runs the task. The decided rule holds only while the server is running, unless the default is changed; the owner must decide.

---

Final note: this output contains proposals and decisions as stated in the validated designs; nothing here was built, run, timed or measured. Spot checks made in this pass: the `ureq` entry in `[workspace.dependencies]` (native-tls, cookies); schema.rs:377 (synchronous NORMAL); schannel.rs:360 (chain FIXME); borders.rs:22-25 (set_border_mark uses INSERT OR REPLACE). Everything else is taken from the validated designs and their validators' corrections, with citations as stated by those reviews.
