# Incremental sync for the web and phone client

Tracks the task of keeping clients in sync with the catalog incrementally. Full resyncs are allowed only for the first
load and for a restored or rebuilt catalog. Nothing here is built. Decisions are recorded in
`WEB_CLIENT_TECH_DECISIONS.md`; this file tracks the work and the open points.

**Status:** validated (2026-11-14). The recommendation of approach 7 is withdrawn: its cost and full-resync claims
do not hold. The owner approved relaxing the per-listing constraint (chat, 2026-11-14). A window-fetch proposal (no
client copy of a list) is pending adversarial validation; the owner said yes to it only after that validation. See
"Later decisions" below.

## Goal

A client holds a copy of each listing it shows (an ordered list of file ids, 16 bytes per item). The server sends
changes since the client's last seq. A client that has been away for any length of time still syncs incrementally.

## Constraints

- **Any listing.** A listing is any folder, filter and sort combination. The server can't keep a sync structure per
  listing, so each changed file is tested against the listing's condition.
- **No cliff by time.** Retention is by what the clients have acknowledged, never by age.
- **Bulk changes.** Zone and layout changes touch every row and have no per-row stamp.
- **Scale.** The catalog is planned for 70M files, and the bounded-memory plan targets 1B. Anything that scales with
  the catalog on every sync is too costly.

## Approaches

| # | Approach | Summary | Main cost |
|---|---|---|---|
| 1 | Stamps and tombstones (current design) | Per-row seq on change; a delete trigger writes a tombstone; delta = stamped rows plus tombstones since seq | Every membership, order and shape write must stamp; retention needed |
| 2 | Change journal | Every mutation appends an operation record; delta replays since seq | One row per change, no coalescing; same risk of a missed write |
| 3 | Per-device outbox over WebSocket | Server queues changes per device; pushes; trims on ack | Per-device state and fan-out; cap needed for sleeping devices |
| 4 | Chunk digests | Client compares digests per chunk; fetches differing chunks | Cost scales with the catalog; a digest per listing can't be kept |
| 5 | Frozen server snapshot | Server keeps the frozen list per device and listing | Server memory per listing; expiry means full resync (rejected earlier) |
| 6 | Key-addressed or index-addressed deltas | Cuts across the others. Key: (order_key, FileId). Index: final positions | Index deltas renumber every later row on insert |
| 7 | Hybrid: 1 as the engine, 4 as backstop | Stamps and tombstones with ack-based retention; digest checks periodically and after bulk events | Two mechanisms; stamping coverage still to fix |

| # | For | Against |
|---|---|---|
| 1 | Cost scales with changes; each id once (stamps coalesce); works for any listing; no time cliff | Stamp coverage must be complete; tombstones need ack-based retention; bulk events need a marker |
| 2 | Exact history; does not depend on stamps being complete | No coalescing (a phone replays every edit); same risk of a missed write as a missed stamp |
| 3 | Real-time; simple client logic | Per-device state and fan-out; a sleeping device's queue grows to a cap, then a fallback. A shared log with a cursor per device gives the same behaviour with less state |
| 4 | No logs or tombstones; catches bulk changes and any drift; self-healing | Cost scales with the catalog: about 17k digests (about 280 KB) per check at 70M, even with no changes. A digest per listing can't be kept, so each check scans the listing (tens of seconds at 70M, an estimate from DATABASE.md) |
| 5 | Exact diff | Server memory per listing per device; expiry brings back full resync |
| 6 | Key: independent deltas, safe to reorder or batch; sorted-structure apply. Index: flat-array apply | Index: an insert renumbers every later row |
| 7 | Cost scales with changes; works for any listing; no time cliff; the backstop covers missed stamps | Two mechanisms to build and test; stamp coverage still to fix |

## Recommendation

Approach 7, with key-addressed deltas, if the owner agrees to (6). The only full transfer is the first load and recovery
from a restored catalog.

## Decisions so far

| Item | Status | Where |
|---|---|---|
| Client-held replica (O16, option A) | Accepted as recommended. Amends the decided listing stamp | `WEB_CLIENT_TECH_DECISIONS.md` |
| Migration 10 after cutover (O17, option c) | Accepted, **but conflicts with the bounded-memory plan's Phase 2 freeze** (see Open) | Same |
| Tombstones are the removal signal | Agreed. A replica keeps showing a deleted file without one | Same |
| Time-based tombstone retention (90 days) | **Replace** with acknowledged-seq retention. Recommended, not accepted | Same, "Sync" section |
| Migration 10 keeps `files.id` and `seq` values | Recommended | Same |
| `listing_key` as an explicit, versioned encoding | Recommended. The key doesn't exist in code yet | Same |
| Quarantine dropped; delete goes to the Recycle Bin | Accepted. No purge command | Same |

## Open

1. **Key-addressed deltas (approach 6).** Recommended yes. Owner to confirm.
2. **Change history.** Does the owner need undo or an audit trail? If yes, approach 2 earns its cost.
3. **Migration 10 vs the bounded-memory plan.** The plan's Phase 2 is one BASELINE schema edit, then a freeze (D1). After
   the freeze, a `files` rebuild takes 5–9 h at 1B files (`docs/plans/BOUNDED_MEMORY_PLAN.md`, migration table). Options:
   fold the sync schema into the Phase 2 BASELINE before the freeze (recommended), or accept an exclusive rebuild after
   cutover. This changes the accepted O17 decision.
4. **Acknowledged-seq storage.** The server records each device's last applied seq. Write it on every poll, or
   batch it (hourly, as the device design does for last-used)?
5. **Digest backstop cadence.** Periodic, after bulk events, or both? Which listings?
6. **Bulk changes.** Full resync (rare), or digests catch them (approach 4)?
7. **Stamping coverage.** Gaps listed in `WEB_CLIENT_DESIGN.md` (around line 473) and `WEB_CLIENT_RECONCILED.md` (lines
   85–86): `ratings.rating`, `file_writes.value`, `border_marks`, `analysis.error`, and UPDATEs of `created`, `folder_id`,
   `name`, `ext`, `order_key`. Delete paths `pair.rs:345` and `pair.rs:434` need a tombstone trigger.
8. **FileId reuse.** `files.id` is `INTEGER PRIMARY KEY` with no AUTOINCREMENT (`crates/catalog/src/schema.rs`, around
   line 549). A deleted id can be reused, which would confuse a replica. Migration 10 fixes it if approved.
9. **`listing_key` build scope.** Derived from the serialised Listing, any serde change orphans every replica on upgrade.

## Next steps (superseded by the tasks section)

- [ ] Owner answers open questions 1, 2 and 3.
- [ ] Decide acknowledged-seq storage and digest cadence (4, 5).
- [ ] Write the stamping-coverage test: a model test that interleaves writes and compares a replica with a fresh read
      (design: "Model test", `WEB_CLIENT_DESIGN.md` around line 530).
- [ ] Add the sync schema to the bounded-memory plan's Phase 2 BASELINE, if approved.
- [ ] Add the folder-browser task to `docs/plans` (see `WEB_CLIENT_TECH_DECISIONS.md`, B11).
- [ ] Move this file to `docs/plans` once the owner accepts an approach.

## Sources

- `docs/temp/WEB_CLIENT_TECH_DECISIONS.md`: decisions and spike results.
- `docs/temp/WEB_CLIENT_DESIGN.md`: listing snapshots and sync, around lines 442–530; tombstones around line 482.
- `docs/temp/WEB_CLIENT_RECONCILED.md`: section 4.6 (stamps, tombstones, bulk marker), around lines 972–1010.
- `docs/plans/BOUNDED_MEMORY_PLAN.md`: Phase 2 (BASELINE), decision D1 (freeze), migration table.
- `docs/plans/DATABASE.md`: scale figures for counts and scans.

## Validation (2026-11-14, adversarial agent, every claim checked)

Verdicts: 1 weakened; 2 weakened; 3 weakened; 4 refuted on cost; 5 weakened; 6 weakened; 7 refuted as written.

Measured on a 2M-row SQLite table, extrapolated linearly to 70M (in-cache, Python harness, upper bounds):
- ordered scan, no join: 73 s; with a sha256 chunk digest: 110 s; with the analysis join: 475-508 s.
- BLAKE3 at 3.5 GB/s is 2 s per 70M hashes; sha256 at 0.48 GB/s is 18 s.

Findings that change the picture:
- The current Delta is one O(N) server pass per sync (RECONCILED 4.4, line 958). "Cost scales with changes" is false.
- Approach 7 forces Full in six ways: bulk_seq (including the AUTOINCREMENT rebuild), the 90-day floor, the global-change
  bound (|C| over 50,000, or over 10% of the listing count, counting unrelated changes), digest mismatch, replica
  eviction, and the LAN budget (5M items per minute: 14 min for a first load at 70M, 200 min at 1B).
- Retention cliff: a signed-out device still counts as non-revoked. The floor either pins forever or releases after 30
  days, which is a Full on return.
- Restore is not detected by catalog_id: rebuild and backup keep it (rebuild.rs:159-161, 204-208).
- Tombstone scope defect: a row moved out of a listing and then deleted is never reported as left. Not reachable today;
  reachable once user deletes land.
- Stamp gap list was stale. Missing: unpaired 1 to 0 (pair.rs:204), kind, the bulk pending flip (lifecycle.rs:74-86),
  analysis width, height, taken, orientation, frames, duration_ms.
- Key-addressed deltas need sort values in the record; client memory for id plus order_key is 9 GB at 70M, 129 GB at 1B.
- Client memory for the index replica: 1.12 GB at 70M, 16 GB at 1B, per listing.
- Session extension: needs every writer attached; does not solve retention or bulk.
- Version vectors and CRDTs: refuted for this problem.
- Per-listing materialised server structures: survive if the "no per-listing server state" constraint is relaxed. The
  bounded-memory plan's grid already keeps per-listing sorted runs on disk (BOUNDED_MEMORY_PLAN.md line 78), refreshed in
  O(changes x log n). Not tested for sync.

Conflicts across the docs (the full list is in the validation report): the bounded plan caps reads at 2 s (D31); the design
puts a 70M pass at 73-110 s; RECONCILED 4.9 still specifies POST polling where TECH O27 accepted WebSocket; the set digest
is O(changed) in one place and O(N) in another; migration 10 "keeps ids so no resync" conflicts with bulk_seq.

## Further findings (review, not yet recorded above)

- **Removal lag.** The bounded-memory plan hides pending removals from readers at once (`BOUNDED_MEMORY_PLAN.md` D34), but
  a removal takes 3 to 80 h (its migration and removal table). The sync stream sees a removal only when the rows are
  stamped `missing`, so clients keep showing removed files for hours unless the mask is part of the stream.
- **Keyset paging.** `DATABASE.md` (lines 48-50, 191) says the grid must page with keyset paging. RECONCILED 454 rejects
  keyset paging. The sources disagree.
- **Device cap.** RECONCILED 604: 32 non-revoked browser devices. Signed-out devices still count, so the outbox of approach
  3 can hold up to 33 devices of state per change, indefinitely.
- **Chunk digests (approach 4).** Position-based chunks change every chunk after an insert. Key-range chunks need a tree
  per listing, which the server cannot keep. A global order-key tree does not match filtered or sorted listings.
- **Write amplification (approach 2).** Counted from `writers.rs` and `schema.rs`: a stamped change costs 2 statements per
  trigger firing (`sql.rs:25-45`). A journal row costs about the same per change. The difference is coalescing, not cost.
  A bulk change is one row per affected row for a journal, against one marker for `bulk_seq`.
- **Frozen server snapshot (approach 5).** The server holds 16 B per item per listing per device: 1.12 GB per listing at
  70M, 4.48 GB per device at the proposed cap of four, 16 GB per listing at 1B. The 56-byte figure is the desktop `Entry`
  (`crates/model/src/grid.rs:41`), not a server cost.
- **Client memory.** One index replica is 1.12 GB at 70M. Two replicas plus a spare for the merge is about 3.4 GB on the
  phone, against the bounded plan's grid target of about 11 MiB client (`BOUNDED_MEMORY_PLAN.md:78`).
- **Session extension (added approach).** Not enabled: rusqlite is built with `bundled, collation, fallible_uint,
  functions, hooks` (`Cargo.toml:39`). It needs every writer connection attached, and does not solve retention or bulk.
- **Model results.** `scratchpad\semantics\model.py` (Python model of RECONCILED 4.4, not the Rust code): S1 unpaired flip
  diverges; S2 unstamped rename diverges; S3 move-then-delete diverges; S4 bulk pending flip diverges; S0, S2b, S3b (in-scope
  delete) converge. The SQLite behaviour is in `scratchpad\semantics\sem.py` (REPLACE fires the insert half only, upserts
  fire only when the WHEN holds, cascades fire child triggers, rowid is reused without AUTOINCREMENT).
- **Open questions, by who answers them.** 1 (key-addressed): owner, with client memory from the arithmetic above (9 GB at
  70M). 2 (history): owner only. 3 (migration 10): owner, plus the 5-9 h figure, which is the plan's estimate, not
  measured. 4 (ack storage): owner, with a write-rate measurement not yet taken. 5 (digest cadence): owner, now bounded by
  the measured 73-110 s pass at 70M. 6 (bulk): owner, measured Full cost of 1.12 GB at 70M. 7 (stamp coverage): code, the
  list above. 8 (FileId reuse): code, and already decided by the owner (TECH 197-198). 9 (`listing_key`): code and owner.

## Decisions (2026-11-14)

1. **Relax the constraint (approved by the owner in chat, 2026-11-14).** The server may keep per-listing state. Deltas
   are computed from per-listing sorted runs, not from an O(N) scan. Later proposals may still change this.
2. **Write the stamp and tombstone fixes as tasks now (approved by the owner in chat, 2026-11-14).** Done below. They are
   needed under any approach.

## Later decisions (applied on top, 2026-11-14)

The agent's findings above are kept as written. These are the later decisions, applied on top of them.

1. **Window-fetch direction: proposed, not accepted.** The owner said yes to it, but only after adversarial validation.
   Validation of the ten claims and twelve checks (P1–P10, A–L) is in progress. The proposal:
   - The phone holds no copy of a listing. It requests the window it shows, by key, from the server's per-listing run.
   - Removed files are absent from the answer. No tombstone is needed for the list.
   - The only sync signal is "listing L changed" (a generation per listing). Bulk events bump the generation, so no bulk
     marker is needed.
   - Scroll anchors are keys, which needs keyset paging. The design rejected keyset paging (`RECONCILED:454`), so this
     reverses that decision.
   - Layout comes from server checkpoints every 4096 items (`BOUNDED_MEMORY_PLAN.md:1484`). A removal reflows the rows
     after it, and the bounded plan's frozen tail caps the cost.
   - The owner's question that led here: removing a file from the client on a server 404 is not enough. A 404 is only
     seen when the client asks for that file, and an off-screen removal never reaches the copy, so the count, the scroll
     and the layout stay wrong. Tombstones are a sync record, never displayed.

   If validation holds, the following tasks change or go away: A6 and A7 (tombstones), B3 (floor), B4 (restore detection
   for lists), B5 (paged Full), B7 (replica shape), and D-bulk, D-floor and D-shape. It also reverses the full replica
   (`RECONCILED` 4.8, O16 option A). Until validation returns, all the tasks stay as written.
2. **D-bulk, D-floor, D-shape and open question 3: still undecided.** The recommendations in the tasks section are mine,
   not the owner's.
3. **Relaxing the constraint and the stamp fixes:** approved as recorded above.

## Tasks

### Track A: correctness fixes (any approach)

Each task has an acceptance test. The Python model in the validation (`scratchpad\semantics\model.py`) is the spec for the
replica-equals-fresh-read check until the Rust model test exists (`WEB_CLIENT_DESIGN.md` around line 530).

- **A1. Stamp unpaired 1 to 0.** `crates/catalog/src/pair.rs:227` (`UPDATE files SET unpaired = 0`; line 204 is a SELECT) has no stamp. Add
  `changed` and `seq`. Test: model S1 converges.
- **A2. Stamp the identity upserts.** `writers.rs:113-116` (scanner upsert: folder_id, name, ext, kind, order_key),
  `writers.rs:101` (created backfill), `pair.rs:355-360` (take_identity). A stamp trigger on `UPDATE OF folder_id, name,
  ext, order_key, created` **must carry `WHEN old.x IS NOT new.x`** (the scanner's upsert sets these columns on every unchanged
  rescan, and an unconditional trigger would stamp every file on every rescan). The design's form at `WEB_CLIENT_DESIGN.md:466-468` has the WHEN. Test: rename through upsert reaches the replica (model S2).
- **A3. Stamp kind.** `files_changed` (`schema.rs:766-773`) sets `pending` only. Add the stamp. Test: a kind change
  reaches the replica.
- **A4. Stamp the bulk pending flip.** `lifecycle.rs:74-86` (`set_import_version`) and `lifecycle.rs:164-171`
  (`set_taken_zone`) change many rows with no stamp. Needs decision D-bulk below. Test: model S4 converges.
- **A5. Stamp analysis shape.** `analysis_changed` (`schema.rs:802-817`) omits width, height, taken, orientation, frames
  and duration_ms. `queue.rs:262` swaps width and height on a turn and is covered only by the same transaction's file
  trigger. Add them to the trigger. Test: a turn that changes shape reaches the replica.
- **A6. Tombstone on every delete.** No `AFTER DELETE ON files` trigger exists. The deletes are `pair.rs:342` and
  `pair.rs:431`. Add a tombstone trigger, with a cascade test (`TRIGGER_TESTS`, `schema.rs:873`). Test: model S3 (a
  deleted row that left a listing still reaches the replica).
- **A7. Tombstones are not scoped by listing.** The design tests a tombstone against the deletion-time `order_key` (step 4
  of the Delta). Proposed rule: a tombstone removes the id from any replica holding it, with no scope test. A replica
  checks membership by id, so the scope test adds nothing. Test: model S3 passes.
- **A8. Stale citations.** Fix the design citations: `roots.rs:58-74` should be `add_root`; `remove_root` is at
  `roots.rs:89-100`; `schema.rs:719-726` and `755-758` should be `766-773` and `802-817`.

### Track B: sync architecture (follows decision 1)

- **B1. Spike: per-listing runs as the delta source.** Read the bounded-memory plan's grid section (`BOUNDED_MEMORY_PLAN.md`
  around line 1437 to 1560, and the sorted-run design at line 78). Measure: one change refreshes its run in O(log n) at
  2M rows, and the disk size per listing. Output: a number for disk per listing at 70M.
- **B2. Rewrite the delta.** Replace `RECONCILED 4.4` (the O(N) pass) with run-based deltas. Drop the global `C` bound
  (step 5), which counts changes outside the listing.
- **B3. Retention by acknowledged seq.** Replace the 90-day prune. Floor = minimum seq over non-revoked devices. Revoking
  a device releases its floor. Decision D-floor below.
- **B4. Restore detection.** `catalog_id` survives rebuild and copy (`rebuild.rs:159-161`, `204-208`). Add a rule that
  detects a restored catalog: `seq > meta.seq` catches an older backup only. Decide the rule. Test: restore a backup into
  a running catalog, and check that the client does a Full.
- **B5. Paged Full.** A first load at 70M is 14 min under the LAN budget. Define a paged Full (ranges) with the budget per
  page. This also meets the bounded plan's 2 s read cap (D31).
- **B6. Set digest.** `WEB_CLIENT_DESIGN.md:467` says O(changed); `RECONCILED.md:969` says O(N). Pick one. A commutative
  sum makes it O(changed) on the client.
- **B7. Replica shape.** A full 16 B/item array is 1.12 GB per listing at 70M. The bounded plan targets a window plus
  checkpoints (about 11 MiB client at 1B, `BOUNDED_MEMORY_PLAN.md:78`). Decide: full array or window. Decision D-shape
  below.
- **B8. Notice channel.** `RECONCILED 4.9` still specifies POST polling. The owner accepted WebSocket (TECH O27). Update
  the doc.
- **B9. Migration 10.** Keep `files.id` and `seq` (no `bulk_seq` for the AUTOINCREMENT rebuild), so an upgrade is not a
  Full. Blocked on the owner's answer to open question 3 (fold into the Phase 2 BASELINE).

### Owner decisions needed

- **D-bulk.** For bulk events (A4): a bulk marker that forces Full, or per-row stamps? The bounded plan suppresses the
  changed stamp during upgrades (D8, D20, line 686), so the two conflict. Recommendation: a bulk marker, so the events are
  visible to clients and the upgrade still converges.
- **D-floor.** A device signed out for a year pins the floor forever under B3. Options: (a) owner revokes it; (b) the
  floor ignores devices unseen for N days, and a return forces a Full. Recommendation: (a), with a notice to the owner.
- **D-shape.** Full array (1.12 GB per listing at 70M) or a window with checkpoints (B7). Recommendation: window, since
  the bounded plan requires it.
- **Open question 3** (migration 10 in the Phase 2 BASELINE). Still open.

## Next steps

- [ ] Window-fetch validation: read the result, then decide whether it replaces the replica and tombstone tasks.
- [ ] Owner answers D-bulk, D-floor, D-shape and open question 3 (or they fall away if window-fetch is accepted).
- [ ] A1 to A3, A6 and A7: correctness fixes, each with its acceptance test.
- [ ] B1: spike on per-listing runs.
- [ ] B2 to B9: after B1.
- [ ] A8: doc citations.
- [ ] Move this file to `docs/plans` once the owner accepts the architecture.

## Window-fetch validation (2026-11-14): not ready as proposed

Verdicts on P1–P10: P1 partly; P2 partly; P3 partly; P4 partly; P5 refuted as stated; P6 partly; P7 partly; P8 partly;
P9 partly (rank refuted); P10 partly (restore detection refuted). Scratch: `scratchpad\window_validation\`.

Measured (SQLite 3.49.1, 70M rows, 69,297,918 present, warm OS cache):
- Keyset window of 60: 0.15 ms warm, 0.26 ms cold.
- OFFSET 1,000,000: 10.8 ms warm, 58 ms cold.
- count(*) of the whole listing: 4.1 s. Read cap D31 is 2 s.
- Rank by range walk: 0.28 s, 2.86 s and 5.08 s at 10%, 50% and 90%. Rank is O(rank) in SQLite.
- Per-listing run, key-only: 3.6 GB at 70M measured (the plan's scale gives 5.6–6.7 GB).
- Scroll at 3000 px/s: 0.23–1.4% of the LAN budget. Not the bottleneck.

Layout:
- A 4096-item checkpoint is a row start only 8.2% of the time. A window laid out from a checkpoint re-syncs with the true
  rows after a median of 535 items (p90 1,675, max 6,682). The phone needs aspects between checkpoint and window, which
  the proposal does not send.
- A removal near the top changes y for 99.9% of later rows in the first 2,000 items, and every checkpoint changes.
- Reach figures from the plan (p99 103,112 and 264,079) were not reproduced with my aspect distributions. Unknown.

Consistency:
- Torn reads: four scenarios. A per-window generation check misses a duplicate when the second window is anchored on the
  first window's old last key. A stale window can survive a missed notice until the next change or the timer.
- A missed stamp is silent. The timer refetch reads the run, which never received the change, so it cannot repair it.
  Digests were the only detector, and the proposal removes them.
- A restart without an epoch gives a false "no change" (the generation resets to 0).
- Restore detection is still needed: `catalog_id` survives restore (`rebuild.rs:157-161`, `202-209`).

Anchors:
- Keyset with NULL sort keys drops rows silently. Keyed sorts order by (sort value, order_key, id), so "order key plus
  FileId" is not a valid anchor.
- A rating edit moves the item's own key. Re-anchor rules are needed (O35, O36).

Open choices for the owner (not decided):
- (a) Keep the window design and fix the gaps above: chained windows, aspects in each window, an epoch, and stamp repairs.
- (b) A client copy of aspect ratios only (2–4 B per item, per the plan's ratio blob), with ids in windows. Not validated.
- (c) Return to the full replica, with the tombstone and floor tasks as written.

## Proposed design v2 (2026-11-14): pending adversarial validation

Summary: the server owns a versioned per-listing run and layout; the phone holds no copy of any list; it requests windows
by key, each tagged with the snapshot version; the client never mixes versions. Tombstones, digests, the replica and restore
detection for lists are not needed. This section is the proposal under validation. It is not accepted.

Components:

1. **Layout is server-owned.** A window request carries the viewport width and target height. The server runs the greedy
   layout (`pg-client`, WASM-clean) from the nearest stored row start and returns the window's row geometry. The client draws.
2. **Checkpoints at row starts.** Store the layout state at true row starts, not at fixed item counts, so any window starts
   at an exact row.
3. **Removals are atomic.** The phone replaces the visible window in one frame and keeps the scroll anchor on the same item.
   Exact geometry below the change is recomputed in the background; the old frame stays on screen until the new one is ready.
   The frozen tail is a cost cap only, not the fix.
4. **Counts and ranks are maintained.** The run keeps counts and ranks in a blocked Fenwick tree (as the bounded plan
   specifies), so rank and count are O(log n). The 2 s read cap applies per request; a whole-listing count is never a request.
5. **Snapshot versions.** Every window carries its listing's snapshot version. A window from an older version is refetched
   with the visible set at the newest version. The client never splices two versions.
6. **Write path.** Every membership, order and shape write goes through one function that updates the run in the same
   transaction. Triggers remain as a second line. A background scrub rebuilds a sample of runs from the catalog and diffs them.
   The per-column test in tasks A1–A7 applies.
7. **Epoch.** Each server start gets a new epoch, included in every version. A changed epoch forces a window refetch.
8. **Restore.** Holds nothing per list, so a restored catalog (new epoch) costs one refetch of the visible windows.
9. **One order definition.** The order is defined once in `pg-model`. The ORDER BY and the keyset predicate are generated
   from it. NULLs are placed explicitly (last, in both directions, as the catalog does now). Tests cover NULL keys, both
   directions and ties, against the real query.
10. **Rating edit moves the anchored item.** The viewport moves to the next surviving item in the old order (O36).
11. **Attribute-only changes.** The notice carries a per-item version (`ItemChanged`, design line 502). Only that item's
    attributes update. Layout changes only when shape or sort key changes.
12. **Stamp gaps.** Tasks A1–A7 as written. Everything above depends on them.
13. **Notice channel.** WebSocket (O27). Correct RECONCILED 4.9, which still says POST polling.
14. **Desktop.** The desktop moves to the same windowed reader, in-process. One code path for both clients.
15. **Facets.** Facet counters are kept in the run's transaction (DATABASE.md, section 3, option 1). No GROUP BY per request.
16. **Cold listings.** A listing opened once is served by a direct keyset scan while its run builds in the background. Runs are
    derived caches with an LRU and a byte cap, rebuilt on demand.
17. **Transport.** HTTP/2 over TLS (required by the LAN design), so window requests multiplex and do not compete with images.

Open for validation: whether the epoch rule holds across crashes; whether layout checkpoints at row starts are stable under
the width and height the client sends; the cost of background exact re-layout after a removal; the scrub's sample size and
its cost; whether one order function can serve both the desktop's in-process grid and the server.

## Design v2 validation (2026-11-14): not ready

Read-only review of "Proposed design v2". Scratch: `scratchpad\design_v2\`. Verdicts: C1 flawed; C2 flawed; C3 flawed under
churn; C4 unsupported; C5 unsupported; C6 flawed; C7 sound; C8 flawed; C9 sound (mechanism), doc tie wording flawed; C10 flawed;
C11 flawed; C12 flawed; C13 flawed; C14 unsupported; C15 flawed; C16 flawed; C17 unknown.

Key measured results:
- Layout: a window's y is a prefix sum over all earlier rows, so "lay out from the nearest row start" gives neither y nor total
  height. Row starts stay valid only for the same (W, T): a change to (1280,220) keeps 21–22% of row starts; (1080,230)
  keeps 39–41%. RowHeight has 741 values and pinch is continuous.
- Re-layout cost: a 70M pass is 0.25–0.34 s in RAM (Rust scratch). Under churn of 2 changes/s per (W,T) per listing it is
  0.5–0.7 of a core per listing.
- Snapshot windows: 3 windows with 0.3 s read latency, 50 changes/s: 108 of 800 trials did not converge in 200 rounds.
- Fenwick (BMP:1469-1470) is over a fixed slot universe and says nothing about inserting new keys mid-run.
- Stamps: A2 as written stamps every file on every rescan (corrected above). A1 and A6 citations corrected above.
- Scrub: no sample size stated; detects a missed row only if its listing is sampled.
- Keyed sorts cold at 70M (2M measured, scaled linearly; likely low): size ~10 s, taken with join ~63 s, count 4.1 s
  (exceeds D31's 2 s). Run size 9.77 GB with the 121-byte average keys (3.22 GB with 32-byte keys).
- Keyset predicate: generated predicate 0 mismatches in 120 runs per sort; naive row-value predicate mismatches all keyed
  runs (10,020 rows missing asc; 30,714 missing and 1,786 duplicated desc). Ties: name desc is (order_key DESC, id DESC),
  keyed sorts are (order_key ASC, id ASC). REC:960 is wrong.
- WebSocket: a per-connection "after the listing's last delivered S" state needs a send buffer, which is a queue. REC:270
  still rejects a queue; REC:1003-1011 still specifies POST.
- Desktop: `crates/client/src/grid.rs` holds a whole Vec<Entry>; position_of, relayout, refresh_turned and selection_after
  all scan it. v2 says "in-process" where DES:521-523 says loopback.
- Facets: counts are given the other facets, so filtered combinations are not bounded. DB:195-196 says filtered views scan.

Owner decisions conflicted: O16 (replica, TECH:135-136), O17 and the BMP freeze (BMP:103), tombstones stay (TECH:201-204),
O27 (REC:270 still rejects), O35/O36 (misapplied to a move), D31 (2 s cap), D34 (removal mask, unaddressed), BMP:90
(constant-in-n bound), DB:195-196 (filtered facets), REC:454 (keyset, acknowledged). No conflict with the hash, NTFS,
journal or Turso decisions.

Trade not stated by v2: it removes the digests (the only detector of missed stamps) for a sampled scrub, and adds per-(listing,
sort, filter, W, T) server state of 3.2–9.8 GB per listing at 70M. The phone replica (1.12 GB at 70M, REC 4.8) is
comparable in size and verifiable.
