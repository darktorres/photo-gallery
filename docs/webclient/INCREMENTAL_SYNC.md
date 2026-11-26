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

| # | Approach                                | Summary                                                                                                    | Main cost                                                              |
|---|-----------------------------------------|------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------|
| 1 | Stamps and tombstones (current design)  | Per-row seq on change; a delete trigger writes a tombstone; delta = stamped rows plus tombstones since seq | Every membership, order and shape write must stamp; retention needed   |
| 2 | Change journal                          | Every mutation appends an operation record; delta replays since seq                                        | One row per change, no coalescing; same risk of a missed write         |
| 3 | Per-device outbox over WebSocket        | Server queues changes per device; pushes; trims on ack                                                     | Per-device state and fan-out; cap needed for sleeping devices          |
| 4 | Chunk digests                           | Client compares digests per chunk; fetches differing chunks                                                | Cost scales with the catalog; a digest per listing can't be kept       |
| 5 | Frozen server snapshot                  | Server keeps the frozen list per device and listing                                                        | Server memory per listing; expiry means full resync (rejected earlier) |
| 6 | Key-addressed or index-addressed deltas | Cuts across the others. Key: (order_key, FileId). Index: final positions                                   | Index deltas renumber every later row on insert                        |
| 7 | Hybrid: 1 as the engine, 4 as backstop  | Stamps and tombstones with ack-based retention; digest checks periodically and after bulk events           | Two mechanisms; stamping coverage still to fix                         |

| # | For                                                                                                | Against                                                                                                                                                                                                                           |
|---|----------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 1 | Cost scales with changes; each id once (stamps coalesce); works for any listing; no time cliff     | Stamp coverage must be complete; tombstones need ack-based retention; bulk events need a marker                                                                                                                                   |
| 2 | Exact history; does not depend on stamps being complete                                            | No coalescing (a phone replays every edit); same risk of a missed write as a missed stamp                                                                                                                                         |
| 3 | Real-time; simple client logic                                                                     | Per-device state and fan-out; a sleeping device's queue grows to a cap, then a fallback. A shared log with a cursor per device gives the same behaviour with less state                                                           |
| 4 | No logs or tombstones; catches bulk changes and any drift; self-healing                            | Cost scales with the catalog: about 17k digests (about 280 KB) per check at 70M, even with no changes. A digest per listing can't be kept, so each check scans the listing (tens of seconds at 70M, an estimate from DATABASE.md) |
| 5 | Exact diff                                                                                         | Server memory per listing per device; expiry brings back full resync                                                                                                                                                              |
| 6 | Key: independent deltas, safe to reorder or batch; sorted-structure apply. Index: flat-array apply | Index: an insert renumbers every later row                                                                                                                                                                                        |
| 7 | Cost scales with changes; works for any listing; no time cliff; the backstop covers missed stamps  | Two mechanisms to build and test; stamp coverage still to fix                                                                                                                                                                     |

## Recommendation

No approach is recommended as final. Approach 7 is withdrawn (see Status). Key-addressed deltas (approach 6) are
recommended, pending the owner's confirmation (Open, item 1). The window-fetch proposal (Later decisions, item 1) is the
candidate under validation. The only full transfer is the first load and recovery from a restored catalog.

## Decisions so far

| Item                                               | Status                                                                               | Where                          |
|----------------------------------------------------|--------------------------------------------------------------------------------------|--------------------------------|
| Client-held replica (O16, option A)                | Accepted as recommended. Amends the decided listing stamp                            | `WEB_CLIENT_TECH_DECISIONS.md` |
| Migration 10 after cutover (O17, option c)         | Accepted, **but conflicts with the bounded-memory plan's Phase 2 freeze** (see Open) | Same                           |
| Tombstones are the removal signal                  | Agreed. A replica keeps showing a deleted file without one                           | Same                           |
| Time-based tombstone retention (90 days)           | **Replace** with acknowledged-seq retention. Recommended, not accepted               | Same, "Sync" section           |
| Migration 10 keeps `files.id` and `seq` values     | Recommended                                                                          | Same                           |
| `listing_key` as an explicit, versioned encoding   | Recommended. The key doesn't exist in code yet                                       | Same                           |
| Quarantine dropped; delete goes to the Recycle Bin | Accepted. No purge command                                                           | Same                           |

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
   `name`, `ext`, `order_key`. Delete paths `pair.rs:601` (`rebind`) and `pair.rs:686` (`replace`) need a tombstone trigger.
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

- `docs/webclient/WEB_CLIENT_TECH_DECISIONS.md`: decisions and spike results.
- `docs/webclient/WEB_CLIENT_DESIGN.md`: listing snapshots and sync, around lines 442–530; tombstones around line 482.
- `docs/webclient/WEB_CLIENT_RECONCILED.md`: section 4.6 (stamps, tombstones, bulk marker), around lines 972–1010.
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
- Stamp gap list was stale. Missing: unpaired 1 to 0 (pair.rs:303), kind, the bulk pending flip (lifecycle.rs:74-86),
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
  functions, hooks` (the `rusqlite` entry in `[workspace.dependencies]`). It needs every writer connection attached, and does not solve retention or bulk.
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

- **A1. Stamp unpaired 1 to 0.** `crates/catalog/src/pair.rs:303` (the batch's `UPDATE files SET unpaired = 0`) has no stamp. Add
  `changed` and `seq`. Test: model S1 converges.
- **A2. Stamp the identity upserts.** `writers.rs:113-116` (scanner upsert: folder_id, name, ext, kind, order_key),
  `writers.rs:101` (created backfill), `pair.rs:609-616` (the take_identity statements). A stamp trigger on `UPDATE OF folder_id, name,
  ext, order_key, created` **must carry `WHEN old.x IS NOT new.x`** (the scanner's upsert sets these columns on every unchanged
  rescan, and an unconditional trigger would stamp every file on every rescan). The design's form at `WEB_CLIENT_DESIGN.md:466-468` has the WHEN. Test: rename through upsert reaches the replica (model S2).
- **A3. Stamp kind.** `files_changed` (`schema.rs:766-773`) sets `pending` only. Add the stamp. Test: a kind change
  reaches the replica.
- **A4. Stamp the bulk pending flip.** `lifecycle.rs:74-86` (`set_import_version`) and `lifecycle.rs:164-171`
  (`set_taken_zone`) change many rows with no stamp. Needs decision D-bulk below. Test: model S4 converges.
- **A5. Stamp analysis shape.** `analysis_changed` (`schema.rs:802-817`) omits width, height, taken, orientation, frames
  and duration_ms. `queue.rs:262` swaps width and height on a turn and is covered only by the same transaction's file
  trigger. Add them to the trigger. Test: a turn that changes shape reaches the replica.
- **A6. Tombstone on every delete.** No `AFTER DELETE ON files` trigger exists. The deletes are `pair.rs:601` and
  `pair.rs:686`. Add a tombstone trigger, with a cascade test (`TRIGGER_TESTS`, `schema.rs:402`). Test: model S3 (a
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

Measured (SQLite 3.49.1, 70M rows, 69,297,918 present, warm OS cache; the run date is not recorded in the source, and the bundled SQLite is now 3.54.1 at fork rev ac441bb, so these numbers are from an older build):

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
- Desktop: `crates/client/src/grid.rs` holds a whole `Vec<Entry>`; position_of, relayout, refresh_turned and selection_after
  all scan it. v2 says "in-process" where DES:521-523 says loopback.
- Facets: counts are given the other facets, so filtered combinations are not bounded. DB:195-196 says filtered views scan.

Owner decisions conflicted: O16 (replica, TECH:135-136), O17 and the BMP freeze (BMP:103), tombstones stay (TECH:201-204),
O27 (REC:270 still rejects), O35/O36 (misapplied to a move), D31 (2 s cap), D34 (removal mask, unaddressed), BMP:90
(constant-in-n bound), DB:195-196 (filtered facets), REC:454 (keyset, acknowledged). No conflict with the hash, NTFS,
journal or Turso decisions.

Trade not stated by v2: it removes the digests (the only detector of missed stamps) for a sampled scrub, and adds per-(listing,
sort, filter, W, T) server state of 3.2–9.8 GB per listing at 70M. The phone replica (1.12 GB at 70M, REC 4.8) is
comparable in size and verifiable.

## Design v3 (2026-11-14): fix of v2, pending adversarial validation

v2 stays above as written. v3 answers each verdict. It keeps v2's direction: the server owns the per-listing structure,
the phone holds no copy of any list, and windows are requested by position within one listing version. It changes
how the server keeps the structure and how changes reach it. Numbers marked **[measured]** come from the v2 scratch
runs; everything else is a design estimate or a test still to write. Component numbers are v2's; verdict letters are
from the v2 validation.

Shape in one paragraph: for each listing (folder, sort, filter) the server keeps a **run**, a persistent counted B-tree
of file ids in sort order. Triggers on every table that can change a listing write one row per change into `sync_log`
inside the same transaction. A run maintainer consumes `sync_log` in order, re-derives each touched file from the
catalog, and commits a new immutable run version. For each active (run version, W, T, spacing) the server keeps a
**layout**, a persistent counted B-tree of rows (first, count, height). A window request names a listing version and an
item range; the reply is one frame from one version. Clients never splice versions and never hold a list.

Components:

1. **Layout (C1: flawed).** A row's boundaries come from the greedy in `crates/client/src/layout.rs:38-53`: a row closes
   when sum(aspect) x T + gaps >= W. A row's height is (W - gaps) / sum, so heights vary and y is a prefix sum. v3 stores
   the layout as a counted B-tree of rows with subtree sums of count and height. Item to row is a count descent; row to y
   is a height prefix. There are no item-count checkpoints.
2. **Checkpoints (C2: flawed).** Removed. The layout tree is exact at every row, so no start point has to be snapped to a
   true row start. The layout tree is per (W, T) and is built on demand, so the v2 point that checkpoints hold for one
   (W, T) only no longer matters.
3. **Removal and re-layout (C3: flawed under churn).** A change at item p re-lays out from the old row that contains p,
   runs the greedy until a new row boundary equals an old boundary after p (the sync point), and splices the replaced rows
   into the tree. Cost per change per active (W, T) is O(replaced rows x log n), not a full pass. The sync-point length was
   measured only from 4096-item starts (median 535 items, max 6,682) **[measured]**; from a change point it is **to
   measure**. Under churn, versions are coalesced per listing at most every 250 ms, so a burst of changes costs one splice.
   The frame stays atomic: the client shows the old frame until the new version's window arrives. The frozen tail is not
   used.
4. **Pinch (new, from C1).** RowHeight has 741 integer values (`crates/model/src/size.rs`). During a pinch the client
   scales the current frame (preview only). On release it commits one integer T and requests one window at that T. A
   full pass at 70M is about 0.25-0.34 s in RAM **[measured]**; it runs once per commit, not per gesture step.
5. **Snapshot versions (C5: unsupported).** The run and the layout are persistent trees: a new version copies the changed
   nodes only (about one leaf and its path per change; to measure). The server keeps the current version and every version
   requested in the last 60 s per listing. A window request for a dropped version gets the current version with a
   "moved" flag, and the client re-requests.
   v3 does not require a client to reach the newest version. A view at version V is consistent. The v2 trial measured
   "reach a common version within 200 rounds at 50 changes/s": 108 of 800 trials failed. That requirement is dropped.
   The client shows the last consistent frame until a newer one arrives. At 50 changes/s with 0.3 s latency it runs about
   0.3 s behind, which is an accepted cost, not a fault.
6. **Capture and write path (C6: flawed; C12: stamp gaps).** This replaces the per-column stamps (A1-A7) for sync. Triggers
   on every table that can affect a listing (`files`, `analysis`, `ratings`, `border_marks`, `file_writes`, `folders`,
   `roots`, and each table a listing filters or sorts on) fire AFTER INSERT, UPDATE and DELETE with no column list and no
   WHEN. Each writes `(seq, table, file_id, op, old sort tuple, old facet values)` into `sync_log` in the same
   transaction. Old values come from OLD, so a DELETE on a composite-key table (`PRIMARY KEY(file_id, kind)`) still names
   its file. A bulk statement (D-bulk) writes one bulk row, not one row per file.
   The run maintainer reads `sync_log` in seq order and re-derives each touched file from the catalog. Re-derivation is
   idempotent, so a file touched twice is applied once to the new version. Coverage holds for any connection, because the
   triggers are part of the schema. The schema test (verification list) closes the table gap.
   Scrub (C4, C6: the sample-based scrub is removed). Each run is rebuilt from the catalog at server start and on
   eviction, and diffed against the run it replaces. Any difference is recorded as a defect, and the rebuilt run replaces
   the old one. Drift is therefore bounded by the rebuild interval for a long-lived run. The interval is an owner decision
   (O-scrub below).
7. **Log retention (new; replaces D-floor and B3).** `sync_log` rows are deleted once every live run has consumed them.
   There is no device floor, because no client holds a sequence. A sleeping phone costs the server nothing.
8. **Epoch and restore (C7 sound; C8: flawed).** Each server start draws a 128-bit random epoch. Runs are not kept across
   restarts, so each start rebuilds them. Every version, layout and window carries the epoch. A changed epoch makes the
   client refetch its visible windows, which is a few small requests. A restore replaces the catalog file, so it needs a
   stopped server (to verify in `crates/catalog/src/rebuild.rs` and the restore path). `catalog_id` is not used.
9. **One order and filter definition (C9: sound as a mechanism; the tie wording was wrong).** The order and the filter are
   defined once in `pg-model`. The run build SQL, the re-derive predicate and the layout inputs are generated from that
   definition. Ties: name descending is (order_key DESC, id DESC); keyed sorts are (sort value, order_key ASC, id ASC);
   NULLs last in both directions, as the catalog does now. Keyset predicates are not used: windows are positional within a
   version. `RECONCILED:454` (keyset rejected) stands for the client protocol under v3; `DATABASE.md:48-49, 191` (keyset
   recommended) is superseded for this design.
10. **Rating edit and anchors (C10: flawed).** The client's anchor is the item id at the top of the viewport and its
    position in version V. On a new version the server resolves the id: its new position if it is still a member; if it was
    removed, the nearest survivor after its old position (the old version is retained for 60 s). Whether a rating move
    carries the viewport with the item or leaves it at the same position is an owner decision (O35/O36 were written for
    removals, not moves).
11. **Attributes (C11: flawed).** No per-item version and no ItemChanged. Membership and order are versioned. Displayed
    attributes are read at request time. A change to a member's facet or displayed attribute writes a `sync_log` row and so
    bumps the listing's version, and the client refetches its window. This is cheap at human edit rates and removes the
    per-item version store.
12. **Stamp gaps (C12).** A1-A7 are superseded for sync by item 6. The acceptance test for item 6 is a model test: random
    writes through every writer, then the run equals a fresh read, for every listing in the test set. A8 (stale citations)
    stands.
13. **Notice channel (C13: flawed).** WebSocket (O27). The server sends `{listing, version, epoch}` only: the latest version
    per listing, coalesced. No per-connection "last delivered" state and no send queue. A dropped connection loses nothing,
    because the client refetches its visible windows on reconnect. A client with an open listing also refetches its visible
    windows every 30 s, one small request, as a backstop for a dropped notice. This is not a digest. RECONCILED 4.9 (POST
    polling) and REC:270 (no queue) still need correcting.
14. **Desktop (C14: unsupported).** v3 does not change the desktop. It keeps its in-process whole-list grid
    (`crates/client/src/grid.rs`, `Vec<Entry>`). The v2 claim "one code path for both clients" is withdrawn. Moving the
    desktop onto windows is a separate track, not sized here.
15. **Facets (C15: flawed).** Counters live in each run, one set per listing: for each facet dimension, a count per value
    over that listing's members. The log row carries the old facet values, so the maintainer adjusts counters in O(1) per
    touched file per dimension. A count for the open listing is a read of its counters, so filtered views no longer scan on
    each request. Counter memory is small per listing. Building counters is part of the run build.
16. **Cold listings and build cost (C16: flawed).** The scroll bar needs the total height, which needs a full pass over the
    listing's aspects, and keyed sorts need a full sort. So the first open of any listing pays a build. v2's keyed-sort
    estimates at 70M were 10 s (size) and about 63 s (taken with join), extrapolated from 2M rows **[measured at 2M,
    extrapolated]**. The layout pass cold at 70M is not measured. v3 shows the listing as "building" with progress. D31's
    2 s cap applies to reads of built structures; the build is a background job. Owner decision O-cold below.
17. **Transport (C17: unknown).** HTTPS and HTTP/2 as before. Window requests are small, so the phone's HTTP/2 negotiation is
    not on the critical path. Still unmeasured on the phone.

Memory, stated plainly (not yet measured):

- Run leaf entries must carry the sort key for order-preserving descent and for insertion, because a neighbour's current
  key in the catalog may already be newer than the version being built. v2 measured 9.77 GB per listing at 70M with 121-byte
  keys and 3.22 GB with 32-byte keys **[measured, extrapolated]**. v3 proposes a 16-byte order-preserving prefix plus id for
  text sorts, with an exact comparison from the catalog only on prefix ties. Reading the catalog for ties happens after the
  log entry, so this is **unsolved**: the validator must test it. Numeric sorts need 8 B value + 8 B id.
- Persistent trees: about 1.5x the leaf payload with node overhead (estimate).
- Layout tree: about 24 B per row (estimate), per active (W, T) only, LRU capped.
- Counters: dimensions x values per listing, small.

Owner decisions added or changed by v3:

- **O-scrub.** Rebuild-and-diff interval for a long-lived run (recommended: every 24 h, plus at each start and eviction).
- **O-cold.** Accept a first-open build at 70M of tens of seconds for keyed sorts, and an unmeasured layout pass, with
  progress shown (recommended). The alternative, an estimated scroll height, is rejected by the correctness-first rule and
  is listed for completeness.
- **O35/O36 (moves).** Whether a rating move keeps the item or the position in view.
- **D-bulk.** A bulk log row forces a full re-derive of runs (recommended), or per-file log rows.
- **Migration.** `sync_log` and the triggers are additive (no table rebuild), but they touch the Phase 2 BASELINE (D1
  freeze). Recommended: add them to BASELINE before the freeze. This replaces open question 3.
- **D-floor, D-shape, B3, B4, B5, B9, tombstones.** Not needed under v3 (no client sequence, no replica, no tombstone).
  Owner to confirm that the device floor and tombstones are withdrawn.

Verification required before v3 can be called ready (each a test or a measurement, not a claim):

- Schema test: every table is either in the trigger set or on an exempt list with a reason. Fails the build on a new table.
- Model test (item 12) across all writers, including the scanner upsert, pairing, lifecycle bulk and rebuild.
- Sync-point length from a change point, at 2M and at a scaled 70M layout.
- Run build time and disk and RAM per listing at 2M, then scaled, for the text sort with the 16-byte prefix.
- Trigger write overhead on the scan path (`sql.rs:25-45` estimate: 2 statements per firing).
- Restore procedure: confirm a restore cannot run against a live server.
- Connection inventory: every production open of the catalog file (`crates/catalog/src/core.rs:71, 99, 138`,
  `rebuild.rs:105`, and any other process) must be inside the server process, or its writes must fire the triggers.

## Design v3 validation (2026-11-14): not ready

Read-only adversarial validation of "Design v3", all 17 components, the owner-decision conflicts, and the citations.
The validator changed no repo file. Scratch: `scratchpad\sync_check.py`, `sync_big.py`, `trig_check.py`, `same_value.py`,
`overwrite_checkpoint.py`.

Verdicts (v3 answers v2's verdict? then v3 verdict):
1 layout: yes; SOUND. 2 checkpoints: removed; SOUND. 3 removal and re-layout: partly; FLAWED (cost).
4 pinch: new; UNSUPPORTED (new-T tree build not measured). 5 snapshots: requirement dropped; UNSUPPORTED (path-copy not measured).
6 capture: no; FLAWED. 7 log retention: n/a; SOUND if 6 is fixed. 8 epoch and restore: no; FLAWED.
9 one order definition: yes; SOUND as mechanism, the generated SQL not yet written. 10 rating anchor: partly; FLAWED.
11 attributes: yes; UNSUPPORTED ("cheap" not measured). 12 stamp gaps as a test plan: yes; UNSUPPORTED (test not built).
13 notice channel: yes; SOUND on loss (repaired within 30 s), but content conflicts with O27 (TECH:186).
14 desktop: withdrawn; FLAWED (decided loopback and one-step cutover, REC:123, 147; DES:522, unresolved).
15 facets: partly; FLAWED. 16 cold listings: partly; UNSUPPORTED (layout pass and 70M keyed build not measured; conflicts with D31, BMP:133).
17 transport: n/a; UNKNOWN (phone HTTP/2 untested).

Concrete failures:

- Splice (item 3): 0 mismatches in 5,000 random cases (5 settings x 5 aspect distributions x 200, 50 to 2,000 items), and
  0 in 20,000-item cases. Sync points are found quickly for mixed aspects (median 33 to 53 items) but not for a list where 95%
  of items share one ratio (median 1,814 items) or where all aspects are equal (no sync in 200 of 200 at 2,000 items). The
  "O(replaced rows)" claim is false when no sync point exists: the whole tail is re-laid for each change.
- REPLACE: `INSERT OR REPLACE` into analysis (writers.rs:256, 264) and border_marks (borders.rs:54) is production code. The
  catalog never enables `recursive_triggers` (schema.rs:424-425). A REPLACE that changes a value logs only the insert half, so
  the old facet or sort value is lost.
- Same-value UPDATEs fire the trigger (2 of 2 logged). The scanner's upsert (writers.rs:113-116) and `seen` (writers.rs:99)
  touch unchanged rows, so every rescan logs every file and the maintainer re-derives all 70M files.
- Bulk: SQLite triggers are row-level. A bulk UPDATE (lifecycle.rs:77-81) writes one log row per row, not one bulk row.
- Migration rename: SQLite rewrote a trigger body to the old table name on rename; after DROP the insert failed ("no such
  table"). The rebuild path is correct (rebuild.rs:182-189, 236-237: triggers dropped and recreated). The in-place migrations
  (schema.rs:175-176, 242-243) are not. TECH:198 (migration 10, AUTOINCREMENT) is a rebuild, so "additive, no table rebuild"
  in the v3 Migration line is wrong.
- Interleaving (item 2): file N at k0. W1 commits k0 to k1 (log 10, old k0). W2 commits k1 to k2 (log 11, old k1). Batch {10}
  re-derives N from the catalog (k2), deletes k0, inserts k2. Batch {11} deletes by old k1, finds nothing, and inserts k2
  again. N appears twice. The fault is the maintainer reading current catalog values.
- Restore (item 8): there is no restore code (the "restore" hits are UI selection). Windows refuses to rename an open SQLite
  file (WinError 32), but a copy over the open file succeeds. After such a copy, the server's next checkpoint wrote its pages
  back, and a fresh connection read the live state plus a post-restore write, not the backup. The epoch did not change.
- Facets (item 15): the duplicate facet joins the partner's `files.missing` (query/src/lib.rs:75-82), so one flip touches all
  partners. `pdq_pairs` has two file keys and is not in the named trigger set. The border facet is EXISTS over border_sides
  (lib.rs:50-58), which needs per-file, per-class counts, not one OLD row. No tags table or tag facet exists in the repo.
- Staleness (item 13 and 6): a lost notice is repaired within 30 s for a visible listing. A stalled maintainer or a missed log
  row is not repaired by refetch. Runs are rebuilt only at start and eviction, so drift has no bound for a long-lived run.
- Memory (item 7): text-sort run at 70M: 24 B x 69.3M x 1.5 = 2.49 GB. Layout tree: 0.33 to 0.40 GB per (W, T). One run plus
  one layout is about 2.9 GB, against the whole-process budget of 3.62 GiB (BMP:90). A second listing in RAM exceeds it. v3
  stated neither residency nor a byte cap for runs; v2 had both.
- The line "a cheap per-change splice" holds only where a sync point exists.

Owner conflicts (not decidable by the design):

- Replica "withdrawn" conflicts with O16, "Accepted as recommended" (TECH:133-135).
- Tombstones "withdrawn" conflicts with "Tombstones stay" (TECH:201).
- Notice content conflicts with O27 (TECH:186).
- Desktop: REC:147 ("the desktop writes nothing"), REC:123, DES:522 (loopback).
- Decided stamp (REC:115), snapshot lifetime (REC:138), layout in a Web Worker (REC:244) conflict with items 5 and 1.
- TECH:198 (migration 10, AUTOINCREMENT rebuild) contradicts "additive, no table rebuild".
- D31 (BMP:133): a 2 s read cap applies to every catalog read. v3 item 16 exempts the run build; that exemption is an owner call.
- D34 (BMP:136, removal mask): not addressed.
- Device floor: no conflict (TECH:209, not accepted). Still needs owner confirmation.

Citation errors found by the validator:

- `RECONCILED:454` is a route-table row. The keyset rejection is WEB_CLIENT_DESIGN.md:454.
- REC:969 is wrong for the O(N) claim. The O(N) pass is REC:958.
- DES:466-468 is the digest text. It has no trigger WHEN. My earlier A2 correction said it did; that was wrong. The WHEN has to be
  written, not cited.
- `pair.rs:204` is a comment, not a SELECT. The batch's UPDATE is at `pair.rs:303` (it was `pair.rs:227` when this was written).
- `relayout` is not in `grid.rs`.
- `rebuild.rs` has no `catalog_id` text; survival is through the meta copy at rebuild.rs:204-208.
- The `rusqlite` entry in `[patch.crates-io]` overrides rusqlite with a git fork (the features are still those of the `rusqlite` entry in `[workspace.dependencies]`).

Unmeasured claims (each needs a test or measurement):

1. Layout cold pass and tree build at 70M; pinch-commit latency for a new T (sweep 741 values).
2. Sync-point distance on the real catalog aspect histogram at 70M: no-sync rate, p99 rows re-laid.
3. Run residency and bytes per listing at 70M, RSS and disk, for one and two listings, against 3.62 GiB.
4. Path-copy bytes per change, persistent run and layout.
5. Log rows per unchanged rescan, before and after the WHEN fix.
6. Copy a backup over a live catalog (WAL mode included): must be detected after the fix.
7. Touched files per missing-flag flip for paired files at 70M.
8. Keyed-sort build at 70M (replaces the 2M extrapolation).
9. Listing-version bumps per second during import.
10. Phone HTTP/2 (C17).

Overall: NOT READY. Blocking: item 6 (REPLACE, same-value firing, bulk rows, migration rename); item 2 (old-key interleaving);
item 3 (the cost claim); item 8 (restore); item 15 (facets); the drift bound (item 6); the owner conflicts above; the
unsupported memory total (item 7).

## Corrections applied to v3 after the validation (not yet re-validated)

These fix what can be fixed without an owner decision. Each still needs the validator to confirm it.

1. Triggers: UPDATE triggers carry `WHEN` on the watched columns only, comparing NEW to OLD, so unchanged rows log nothing.
2. REPLACE: `writers.rs:256, 264` and `borders.rs:54` become `ON CONFLICT DO UPDATE` upserts. This is a code change and a
   task. Until it is done, REPLACE on a watched table is not covered.
3. Bulk: a `sync_state` singleton flag. A bulk writer sets it inside its transaction; triggers skip per-row logging while it is
   set; the bulk writer writes one bulk row and clears the flag in the same transaction. Cost: one flag read per trigger
   firing.
4. Migrations and rebuilds: drop the watched triggers before any table rename and recreate them after, as `rebuild.rs:182-189`
   does. Applies to `schema.rs:175-176, 242-243`. Test: a rename followed by an insert logs a row.
5. Interleaving: every log row carries the OLD and NEW sort, filter and facet tuples of the file. The maintainer applies log
   entries in seq order from the logged tuples and never reads current catalog values. The analysis trigger reads the file
   columns by subquery to build the file tuple.
6. Facets: a duplicate-partner change re-derives the partner's facet (fan-out bounded by group size, to measure). The border
   facet is re-derived from `border_sides` for that file, per class. `pdq_pairs` joins the trigger set. Tags are removed (none
   exist).
7. Restore: a random `open_token` in `meta`, written at open and read in each transaction. A changed token drops the runs,
   bumps the epoch, and refuses window reads until the server reopens. The restore procedure must stop the server and delete
   the `-wal` and `-shm` files. Copy-over-live is detected but not prevented; whether restore while running is supported is
   an owner decision.
8. Memory: runs are disk-paged with a byte cap and LRU. At 70M the defaults hold one listing resident, not two. The budget
   needs an owner decision.
9. Citations: keyset rejection is WEB_CLIENT_DESIGN.md:454; the O(N) pass is REC:958; the WHEN must be written (not cited);
   `pair.rs:204` is a comment; `relayout` is not in `grid.rs`; `catalog_id` survives via `rebuild.rs:204-208`.
10. Drift bound: O-scrub (24 h) is the recommendation, still an owner decision.

## Owner decisions needed before the next validation

- Replica (O16) versus the v3 window design: which is accepted? O16 is accepted; v3 withdraws it.
- Tombstones: stay (TECH:201) or go (v3)?
- Notice content versus O27 (TECH:186).
- Desktop: keep loopback and the one-step cutover (REC:123, 147; DES:522), or change them?
- Decided stamp (REC:115), snapshot lifetime (REC:138), and the layout Web Worker (REC:244): reopen them?
- Migration 10 is a rebuild (TECH:198). Fold it into BASELINE before the freeze (open question 3), or accept a rebuild?
- Memory: raise the whole-process budget (BMP:90) or cap runs to one resident listing at 70M?
- Restore while running: supported, or stop-only?
- D31 exemption for the run build; D34 removal mask.
- O-scrub interval; O-cold build latency; O35/O36 for rating moves.
