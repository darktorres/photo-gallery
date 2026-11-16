# Plan: the pairing pass, second round

## Status (2026-11-16)

The first round is [PAIRING_PASS_PLAN.md](PAIRING_PASS_PLAN.md); this round works on P-13 in
[PROFILING_PLAN.md](PROFILING_PLAN.md). Each step's measurements are in [PROFILING.md](../reference/PROFILING.md),
"The pairing pass".

| Step | State |
|---|---|
| 0, the baseline | Done: the vector start, `d949748` |
| 1, transposes as a chunk renaming | Done: `7ab62e7` (the renaming), `f8fa6e9` (no per-slice transposes). The single copy of the hashes and `Joiner::original` move to steps 5 and 7 |
| 2, the radix sort by regions | Done: `7ab62e7` |
| 3, the scan kernel | Done: `7ab62e7` |
| 4, the few-owed path | Done: `07d832a` |
| 5, one thread team | Done: `ede61b5`, the team sorts a stage, then scans it (no ring) |
| 6, the pair-loss fix | Done: `0c8401e`, the stored side is every analysis of a file with an identity |
| 7, the write | Done: the set-based guard and the epoch `57bda80`, the single copy of the hashes `b51e6ae`; the epoch kept by you at +2.20% on the import flush |
| 8, the load | Done: the lookup-free load `c5ee017`, SQLite without memory statistics `bed9346`, the split load after it |
| 9, re-tuning | The threads, the load's parts, the store's permute, the few-owed threshold, the cache, `WRITE` and `BLOCK` done; `SLICE` next |
| 10 | Not started |

## Context

The pairing pass finds near-duplicate pictures by PDQ hash after an import. It was rebuilt in the first round
(`d3d307a` .. `a8a7eac`). Then the vector start (`suffix_avx2`, uncommitted) cut the one-thread scan by 18 - 20%.
The open work is tracked as P-13 in `docs/plans/PROFILING_PLAN.md` and measured in `docs/reference/PROFILING.md`
("The pairing pass", "The one-worker scan on the hardware").

This plan covers every remaining optimization of the pass, except storing fewer orientations: one orientation loses
60 of the 5,277 pairs, two lose 33. It also fixes a pair-loss bug the review found in today's code (step 6). The
user's brief is "churn is not a concern, I want MOST proper", which sets these rules:

- **No arbitrary constants.** Every number is derived, or anchored to today's behaviour.
- **No memory-mapped I/O.** A read error would crash the app instead of failing the pass.
- **No memory cap.** Peak commit is recorded with every measurement, as information only.

**Where the time goes today** (all 187,804 pictures owed, one slice, ms):

| Part | 12 threads | 1 thread |
|---|---:|---:|
| Load | 147 | 150 |
| Prepare | 52 | 51 |
| Sort wait | 181 | 679 |
| Scan | 547 | 3,122 |
| Build + compact | ~22 | ~40 |
| Write | 243 | 229 |
| Commit | 35 | 38 |
| **Pass** | **1,234** | **4,327** |

- **The scan's inner loop** takes 12.7 cycles per iteration, against an issue floor of about 6. Most of that gap is
  mispredicts: one at the loop's exit for every (stored entry, probe), 197 M per pass. A wider kernel does not remove
  them.
- **At 12 threads the join waits on the sort.** The thread sorting the query side does 2.56 G cycles, more than any
  scan thread. Inside the sort, the second pass is 24% and the per-stage zeroing 9%.
- **Iterations.** A replay of the loop counts on the real hashes, exact against callgrind, gives 669 M iterations at 4
  lanes and 386 M at 8.

**Critical files:**

- `crates/dedup/src/join.rs`: the join;
- `crates/import/src/pairs.rs`: the pass;
- `crates/catalog/src/work.rs`: the load;
- `crates/catalog/src/writers.rs`: `PairsWriter`;
- `crates/catalog/src/schema.rs`;
- `crates/query/src/lib.rs`;
- `crates/model/src/pdq.rs`;
- `crates/library/examples/pairs_probe.rs`;
- `crates/library/tests/pairs.rs`;
- `crates/cli/src/lib.rs`, `cli.ftl`.

## Invariants

- **The same pairs** as the oracle (`e2e\after_pairs.txt`, 5,277), except after step 6. From then on the oracle grows
  only by pairs involving a file the old load excluded, and each added pair is listed and checked.
- **A picture is marked done only if its analysis row still exists and is unchanged since the load** (hashes and
  quality).
- **Stop, pause and panic reach every thread.** No thread waits forever, and the first panic resumes on the caller.
- **No new `unsafe` beyond the kernels' vector loads** under the existing dispatch. That rules out vector stores
  through pointers, buffers split across threads by raw pointers, and `set_len` on uninitialized memory.
- **Scratch catalog copies only;** the live catalog is never opened.

## How a change is kept

- **Runs.** The new and old builds run alternately, as pairs, on a fresh catalog copy each time, with background CPU
  logged.
  - **6 pairs per workload** (your choice), turbo off (the power plan's maximum processor state left at 99%). The A/A
    pilot put a +1% half-width at about 60 pairs, so at 6 the intervals are wide, and the decision below is set for
    that.
- **Statistic.** The Hodges-Lehmann estimate and its Wilcoxon signed-rank 95% interval, on the paired log-ratios.
- **Workloads:**
  - all owed, at 1 and 12 threads;
  - few owed (300 and 3,000), at 1 and 12 threads;
  - four slices of 50,000, to exercise the overlapped write;
  - for schema or writer changes, the import flush bench (`crates/bench/benches/ir/catalog.rs`).
- **Decision.**
  - **Gated on time** (`keep.py --part time --rule bounded --margin 0.10`): the join's time, and on four slices the
    pass's, since there the write overlaps the next slice's join and a cost can move between them. The load and the
    last write only add the disk's noise.
  - **Kept, at 6 pairs:** no workload the change touches has its interval wholly above zero (a proven loss), each
    interval's upper bound is at most +10%, and the estimates are not worse overall. A workload whose interval is
    neither is **no verdict**: it is rerun, with more or quieter pairs, not kept. Cycles and the parts are reported
    alongside, to explain a result. (The plan's first rule, an upper bound of at most +1% everywhere, needs about 60
    pairs.)
  - **Why not cycles** (the gate until step 5): with the clock fixed, one worker's cycles are its running time, but on
    several workers they sum the threads. Step 5's team ran the same instructions as before and finished the join
    12 - 50% sooner on 12 workers, yet used 9 - 41% more cycles: it fills each core's second thread, which runs
    slower. At 6 workers (one per core) the excess fell to about +3%.
  - **Real gain:** the interval lies wholly below zero. A variant is built only if it shows a real gain somewhere,
    and kept only if it is also kept everywhere.
  - **Choosing between values** (step 9): the simplest value within +1% of the best wins.
  - **Ties** go to the simpler code.
  - **Correctness fixes** are not gated; their cost is recorded.
- **Builds compared.**
  - Two commits, never a dirty tree.
  - Alternatives that can coexist (the pacing rule and its rivals, fixed or per-stage head width) are compared inside
    one binary, through a probe-only switch.
  - The build is `release` (the `lto` profile no longer exists).
- **Scripts.** The keep-rule scripts move to `tools/profiling/pairing/`.

## Step 0: baseline

- **Commit the vector start**, on your go-ahead. The working tree's only changes are mine: `crates/dedup/src/join.rs`,
  `docs/reference/PROFILING.md` and `docs/plans/PROFILING_PLAN.md`.
- **Re-measure the baseline at that commit.** The other session's `Cargo.toml` and rusqlite changes have been
  committed since the first round's timings were taken, and the rusqlite rev affects them.
- **Measure the starting point.**
  - Count the mispredicts, with the counting-mode PMC trace (`pmc.wprp`).
  - Run the A/A pilots.

## Step 1: transposes become a chunk renaming

**Today**, the 16 chunks are diagonals of the 16x16 bit grid (`join.rs:84-95`). Transposing a hash rotates each chunk,
so every slice transposes and permutes its query orientations 4 - 7 (`join.rs:268-271`).

**Change** the permutation's bit order:

- Diagonals 1 - 7 are read by row, and diagonals 16 − d by column.
- Transposing then swaps chunk d with chunk 16 − d, plus a byte swap of chunk 8. Chunk 8 cannot be made invariant,
  because the transpose pairs its cells with no fixed point.
- Every chunk keeps the same grid cells. So one-bit neighbours, candidates, `matched_earlier`, distances and the
  compacted output are unchanged.
- Nothing persisted holds permuted words: blobs are in Meta's order.

**Then:**

- **Removed:** the per-slice transpose, the `transposed` array (up to 24 MB), and `rank`, whose only use is
  `Sides::words` (`join.rs:585`).
- **Where the renaming applies:**
  - `Sides::words` for transposed codes: a u16 shuffle and the chunk-8 swap, on the cold path;
  - `FewChunk::new` and `query_entry`;
  - the sort's head gathers: a transposed entry's chunks arrive in reverse order, so one 16-bit rotate per u32 word.
- **One copy of the hashes.**
  - `Joiner::new` takes `work.hashes` by value and permutes it in place on the thread team (step 5). That array is the
    store.
  - The write guard gets each picture's Meta-order blob from `Joiner::original(i)`, the inverse permutation: about
    200,000 calls per pass.
  - The catalog keeps returning Meta-order hashes and does not learn the join's layout.
- **Tests:**
  - the renaming equals `permuted(&h.transpose())`, on random and real hashes;
  - `original(i)` round-trips exactly.

## Step 2: the radix sort, by ownership

Each stage sorts both sides by that stage's 16-bit chunk value.

1. **Pass 1, per input block.**
   - A block is sized so its input (read once) and its output fit in L2. The L2 size comes from the CPU's report, or
     256 KB, the i7-8700's.
   - Each block task counting-sorts its block by the key's high byte, into a buffer it owns.
   - A record is a `Vec<u32>` of stride `W + 1` (head words and code), plus a `u16` key array, so one buffer serves
     any head width `W`.
   - The store is read once per stage. Today it is read twice (`join.rs:633-655`).
2. **Pass 2, per high-byte region.**
   - Region `r` reads run `r` of every block, in block order, so the sort is stable.
   - It fills a region buffer it owns: records, codes, padding equal to the kernel's lane count, and 257 offsets.
   - Regions are not split. Pass 2 is a linear copy, so even a degenerate value costs only a few hundred µs.
3. **The sorted side is two-level.** `regions[v >> 8]` replaces one flat array.
4. **Buffers are kept per (set, side, region),** each in its own uncontended `Mutex<Option<Buf>>`.
   - They only grow: `resize` with no `clear`, so only the growth is zeroed, and each stage writes a prefix.
   - Padding may hold any value: `suffix_avx2` stops at the bucket's end (`join.rs:735`), and the lane mask drops
     lanes past it.
   - The per-stage zeroing goes.
5. **The scan's work prefix.** The query side's last pass-2 task computes it over the 65,536 query buckets: the work of
   bucket `u` is `Σ_p |s_{u⊕p}| × (setup + ⌈|q_u|/8⌉)`, with both costs taken from the replay.
   - From the prefix it cuts the stage's block boundaries (step 5, item 4) and stores them, with the stage's cursor,
     in the stage's `Arc`.
   - This runs while the previous stage scans.
6. **A variant under the keep rule:** a write-combined pass 1, one cache line per high byte.
7. **`FewChunk::new`** uses the same sort.

**Interim kernel.** In this step today's 4-lane kernel reads the regions. Its head is the chunks after `c` at a fixed
`W = 4` (today's 128 bits). The sort's cost per record byte is measured here, for step 3's model.

**Tests:**

- the new sort equals `sort_into`, kept as the reference, on random, crowded and skewed keys, at several `W`;
- garbage in every padding slot;
- a cross-region probe: a crafted pair whose only near chunk is 0x00FF against 0x01FF.

## Step 3: the scan kernel

**Status: done, kept** (6 pairs against step 2: cycles −14.0% at 1 worker, −14.9% at 12, −23.3% on four slices; no
workload's interval above zero; `PROFILING.md`, "The scan kernel"). Built as below with three departures, each
measured:

- **The flat list (item 3) was built and dropped.** It halved the mispredicts but added 28% more instructions (43.8 G
  against 34.3 G). That won on one worker and lost 7% of cycles on 12, where the hyperthread had already filled the
  stalls. Each stored entry now tests its own suffix in a loop of 8-lane slots (33.9 G instructions, −11% cycles on 12
  workers against step 2).
- **Buckets of at most 8 entries** load their heads and pictures once, and test each stored entry in straight-line
  code, with the ordering rule as a lane mask.
- **The few-owed path** tests hits on full hashes kept beside the query codes, with no head.
- **Not done:** the 16 × 16-bit lane alternative and the replay. The widths were chosen from per-stage measurements
  (`FULL_TEST_COST`) instead.
- **New cost:** the sort wait on 12 workers doubled (130 → 272 ms), because the sorters no longer keep up. Step 5 is
  what removes it.

1. **An exact threshold per stage and probe.**
   - The PDQ threshold is 31, inclusive (`pdq.rs:10`, `join.rs:711`).
   - A pair is reported at its first chunk within one bit (`join.rs:675`). So at stage `c`, every earlier chunk is at
     distance 2 or more, and chunk `c` is at `[p ≠ 0]`.
   - A first test on a set `S` of chunks that excludes `c` may therefore use the threshold
     `31 − [p≠0] − 2·|{e<c} \ S|`. That is necessary and tight per orientation pair, so the pairs and their least
     distance are unchanged.
   - `S` takes the chunks after `c` first, then wraps to the earlier ones.
   - The minimum threshold is 4, and a compile-time assert checks it is never negative.
2. **The head width `W`.**
   - The baseline is a fixed `W = 4`.
   - The candidate is a per-stage `W` from 1 to 7, chosen by a `const fn` cost model. The model adds:
     - the scan's uops per iteration;
     - `consider`'s cost from the replay, times the expected candidates (the binomial tail `P(Bin(32W, ½) ≤ t)`);
     - the sort's cost per byte, times `4W + 6`.
   - It uses a float loop counter, because the lints forbid `as` and `f64::from` is not const.
   - The per-stage `W` is built only if it is a real gain over `W = 4`.
   - The replay on the real hashes checks the model. If near-duplicates move a stage's `W`, the measured near-pair rate
     becomes a term of the model.
3. **The flat loop** removes the per-entry exit mispredicts.
   - **The loop's order.** The outer loop is the query bucket `u`. Its end `qhi` is the same for all its probes.
   - **Runs.** Each probe's stored bucket `u ⊕ p` lies in one stored region, so a bucket's work is split into runs by
     (region, threshold): {p0}, {p1 - p8}, then p9 - p16 each on its own. That makes 10 runs per bucket, about
     10.5 M exits per slice instead of 197 M.
   - **The list.** For each stored entry, the run writes `[u32; 8]` arrays (offsets `s + 8k` and the entry index)
     with `copy_from_slice` into `list.get_mut(cur..cur + 8)`, and LLVM emits the vector stores.
     - The cursor advances by `ceil((qhi − s)/8)`.
     - Each entry writes `8·ceil(qlen/64)` slots, so the list keeps 8 slots of slack.
     - A batch is sized so the list and the query bucket's head words fit in L1D, from the CPU's report (32 KB here).
   - **One iteration tests 8 query entries:**
     - per word: a broadcast of the stored word (a pure load), a query load, an xor, and nibble-LUT counts;
     - then `vpmaddubsw`, `vpmaddwd`, and `vpcmpgtd` against the run's threshold;
     - `vmovmskps` ANDed with `0xFF >> (8 − min(qhi − off, 8))`, so lanes past the bucket's end never count. This needs
       no BMI2, and works for buckets of any size.
   - **Layout.**
     - Stored records are `[u32; W]`: one `get(x)`, then `W` broadcasts.
     - The query side is strided SoA, with one `off + 8 ≤ stride` check per iteration.
   - **Cost and checks.**
     - About 47 fused uops per 8 candidates at `W = 4`, with no per-entry exit. Today it is 23 uops per 4 candidates,
       plus about 17 cycles per exit.
     - The replay counts the exits removed.
     - The PMC count measures the mispredicts before and after.
     - `annotate.py` checks that the list build compiled branch-free.
   - **Lane width.** The replay's lane-slot counts decide between 8 lanes of 32 bits and 16 lanes of 16 bits. 16-bit
     lanes broadcast as a pure load if each word is stored doubled. Only the chosen width is built.
   - **Dispatch.** `W` is dispatched once per run, through a `match` or a per-stage function pointer, never inside the
     loop.
     - Helpers carry `#[target_feature]` and `#[inline]`.
     - This produces about 50 instances, under 100 KB.
4. **`consider`.** It takes the passing lanes, computes the full distance from the store, then calls
   `matched_earlier`.
   - It is `#[cold]`, `#[inline(never)]`, and not generic over `W`.
   - Today the hit path spills two vectors per stored entry.
5. **One iteration space for all kernels.** The scalar and POPCNT kernels run the same lists and thresholds. The old
   AVX2 kernel stays until the new one is kept.

**Tests:**

- the thresholds at their bound, for every stage and probe, plus random pairs;
- every `W` forced at every stage;
- the head packing equals the chunks of `S`, including the wrapping stages and transposed entries;
- the flat list equals the nested loop:
  - a query bucket over 256 entries;
  - buckets larger than a batch;
  - a claim starting inside a probe;
  - all 10 runs.

## Step 4: the few-owed path

**Status: done, kept.** On 300 owed at 12 workers: cycles −32.9% [−35.7, −32.2], scan −41%. Built as below with these
differences:

- The lists hold query indices into one table of codes and full hashes, with no head words (step 3 found that a head
  saves nothing on this path).
- The 16 chunks are built on the join's threads with scoped threads, until step 5's team takes them over.

**Step 1's remainder: done, kept.** The per-slice transposed orientations and `rank` are gone. The sort reads renamed
chunks and a hit renames the hash, with no slice above zero on cycles. The "one copy of the hashes" part
(`Joiner::new` permuting `work.hashes` in place, and `original(i)` for the write guard) moves to steps 5 and 7, where
its users are.

A pass after an import round usually has few owed pictures. With 300 owed, the scan is 56 ms of a 271 ms pass at 12
threads, and each stored entry does about 11 bitmap-word tests per chunk (`join.rs:453-470`).

1. **The query side is expanded into neighbourhoods, for all 16 chunks at once.**
   - Each query entry is placed under its own value and its 16 one-bit neighbours, with its `S` head words inline. An
     earlier version that fetched the full hash on every hit was 2 - 3x slower per hit (`PROFILING.md:691-694`).
   - The probe flag uses the code's OWED bit. That bit is always 1 on the query side, and `owed()` reads only the
     stored code (`join.rs:683`).
   - Each stored entry is read once and tests the 16 bitmaps, which take 128 KB and stay in L2. The offsets are read
     only on a hit.
   - All 16 expansions are live at once: about 10 MB at 300 owed, and about 50 MB at the threshold's maximum.
2. **The candidates are the same.** A query's 17 values are distinct, so each candidate appears once.
3. **Blocks.** The work per stored entry is unknown before the scan, so blocks are cut by entry count, sized to
   today's block latency (step 5, item 4).

**Tests:** the expansion equals the 17 probes, including a query whose neighbour is another query's value, and
symmetric orientations.

## Step 5: one thread team for the join

**Status: done, kept** (6 pairs against step 1's remainder, gated on time: join −14.3% at 2 workers, −11.3% at 4, −20.7%
at 8, −48.9% at 3,000 owed on 12, −14.7% (pass) on four slices, all real gains; all owed level on 1 worker and kept
at 6 and 12; `PROFILING.md`, "One thread team"). Built as below, with these departures, each measured:

- **No ring.** The whole team sorts a stage, then scans it, in one set of buffers. The ring (item 3), with sorts
  first, and then with scans first, was compared with one set at 2, 4, 6, 8 and 12 workers inside one binary and was
  never faster: +0.4 to +4.7% on the join, a proven loss at 8. It removes the threads' wait at a phase's tail, but its
  sorts cost more than that wait, as two sets' memory is twice one's. So the pacing rule (item 6) is: sort, then scan.
- **The sort's buffers as slices.** The first pass scatters into each block's buffers, and the second reads them
  block by block; each buffer is taken as a slice once. Through the block's `Vec`s, every store made the loop read
  the others' bounds again, and the one-worker sort was 10 - 13% slower than step 1's.
- **The thread budget during an overlapped write** (item 10): all of the pass's threads. With one fewer, four slices
  are level (pass +0.3% [−5.8, +5.8]); the write is faster, the join slower by as much.
- **Not done here:** the single copy of the hashes and `original(i)` (item (a) and step 1), with their user, step 7's
  guard; scan blocks of equal work (item 4), the blocks are 128 buckets each, as before; the build in `Joiner::new`
  timed on its own (item 11). The tests cover a stop at any point, a pause while the others wait, a panic in any
  task, and many threads on tiny stages; not `start()`'s priority on every task.
- **Cycles.** On several workers the team uses more cycles than the old split for the same instructions (all owed on
  12: 32.6 G instructions both, cycles +8%): it keeps each core's second thread busy. That is why the keep rule is now
  gated on time.

The team replaces three things:

- the sorter threads (`Tuning::sorters`, `default_sorters`, `PG_PROBE_SORTERS`);
- the per-stage spawns (`run_stage` → `parallel`, about 160 threads per pass);
- the split between sorters and workers.

The caller's thread does no join work. Today it runs prepare, the sorts below 4 threads, and `FewChunk::new`
(`join.rs:261-271`, `308`, `382`), all without `start()`'s priority.

1. **Threads.** `threads` scoped threads. Each installs its panic guard, then runs `start()`.
2. **Waiting.**
   - A generation counter, changed only under the mutex. A thread with nothing to claim reads it, re-checks the
     cursors, and waits while it is unchanged.
   - Every phase change notifies all: a pass done, a set released, a stop, and the join complete. The last one lets a
     join end with idle waiters.
   - After any wake, a thread calls `control()` before claiming.
   - The mutex is taken with `PoisonError::into_inner`.
3. **Ownership.**
   - **Pass 1.** Its tasks deposit their block buffers in `Mutex<Vec<Option<Block>>>`. The last one freezes them into
     an `Arc<[Block]>`.
   - **Pass 2.** Its tasks read through clones of that `Arc` and fill the region buffers they own. The last one builds
     the stage's `Arc<Stage>` (regions, prefix, block boundaries, cursor), publishes it, and releases pass 1's
     blocks.
   - **The ring** holds 2 stage sets, the minimum for one sort and one scan in flight.
   - **Returning a set.** Every holder drops its handle through `Arc::into_inner`. The one that gets the set back
     returns its buffers to their slots and notifies.
   - **Handles are dropped before `control()` and before waiting.** Holding stage c while waiting for sort c+2 would
     deadlock, because that sort needs stage c's set.
   - **The cursor lives in the stage's `Arc`,** so resetting it for stage c+2 cannot be seen by a thread still on
     stage c.
   - **Why not one contiguous side lent out with `split_at_mut`:** the borrow cannot outlive a phase in a long-lived
     team.
4. **Scan blocks of equal work.**
   - Each block holds the work of today's average block, about 0.4 ms (3,122 ms over 8,192 blocks), so a stop lands no
     later than today.
   - The boundaries are precomputed (step 2) and claimed with `fetch_add`.
   - The smallest unit is a range of one query bucket's stored entries, so a crowded bucket is split. A claim that
     starts inside a probe recomputes its start with `suffix_avx2`.
5. **Tasks, by priority:**
   - (a) the permute of step 1, once per join;
   - (b) the next stage's sort tasks, paced;
   - (c) scan blocks.

   Stage c+1's scan may start during stage c's tail. While both are live, scan progress counts both cursors.
6. **Pacing, with no knob.**
   - A thread that could take either kind takes a sort task while the sort is behind. Both kinds count completed work.
   - A thread waits only when it can claim neither kind, so there is no livelock.
   - It is compared, inside one binary, with "all threads sort, then all scan" and with today's split, at 2, 4, 6, 8
     and 12 threads. It must be kept against both at every count. If it is not, the cause is found and the rule is
     fixed; no per-count switch is added.
7. **Stop, pause and panic.**
   - `control()` runs before each claim, never while a task is held. A `false` sets `stopped` under the mutex and
     notifies all.
   - `Joiner::join` takes `abort: &dyn Fn()`. The guard calls it only if `std::thread::panicking()`, because
     `pairs.rs:131`'s flag would also stop later slices. `pairs.rs` passes `|| { abort.store(true); ctl.wake() }`,
     which wakes threads paused in `Control`. Today a panic during a pause hangs the pass.
   - Every handle is joined explicitly, and the first panic resumes, as `join.rs:917-921` does today.
8. **Output.** A thread compacts its buffer when a push would exceed its capacity, and the buffers are merged and
   compacted at the end. `compact` is idempotent, so the output is the same.
9. **The few-owed path** runs on the team too, so `parallel` goes.
10. **The thread budget during an overlapped write.** `threads` against `threads − 1`, on the four-slice workload.
11. **Timings.**
    - Thread-seconds per task kind, labelled as such, plus idle time split into paused and starved.
    - The build in `Joiner::new` is timed on its own; today it runs on one thread, `join.rs:223`.
    - The CLI line (`cli/src/lib.rs:109-110`), the probe (`pairs_probe.rs:59-75`) and `cli.ftl` follow.

**Tests:**

- a stop before, during and after each phase;
- a pause while others wait;
- a panic in `start()`, the permute, a sort or a scan, also while another thread is paused;
- idle waiters at the end;
- many threads on tiny stages, repeated;
- `start`'s priority on every task;
- pacing forced to each extreme gives the same pairs;
- blocks cover every entry once.

## Step 6: fix the pair-loss bug

**Status: done** (a correctness fix, not gated). Built as below:

- **The load.** `PAIRING_WORK` reads every analysis of a reliable quality of a file with an identity
  (`f.volume IS NOT NULL`). `owed` is the `pairs_owed` row and the file present and not pending, `OWED`'s rule.
  `legacy` counts present, non-pending files only.
- **What it adds on the oracle's catalog: nothing.** The scratch copy has no missing, pending or displaced picture
  (187,804 pictures before and after), so the oracle's 5,277 pairs are unchanged on all seven paths.
- **Its cost.** The load reads the same rows, and `f.volume` from the same `files` row it already reads, so the join and
  the write are unchanged. The oracle runs' loads (207 - 238 ms) are in the range of step 5's (194 - 228 ms); they
  were not taken under the keep rule. Step 8 times the load against this commit.
- **Tests:**
  - six end-to-end cases (`crates/library/tests/pairs.rs`): the file moved away and back (the upsert), its volume
    renumbered (a tier-1 rebind, and a tier-2 rebind under another name), a catalog turn undone, the mtime put back,
    and an older build's import version;
  - the load's test (`writers.rs`) covers a missing, a pending and a displaced row, and legacy rows of missing and
    pending files;
  - "no duplicates" waits exactly while `has_owed_pairs` is true (`browse/read/tests.rs`).

**The bug, in today's code.** The load excludes missing and pending files from both sides (`work.rs:95`). A picture
paired while file Y is excluded is marked done without Y. Y can then come back without a re-analysis:

- from missing, through the scan's upsert (`writers.rs:119`) or a rebind (`pair.rs:379`);
- from pending:
  - a rotation undone (`queue.rs:357-358` → `files_changed`, `schema.rs:820`);
  - an mtime that reverts;
  - two builds alternating `set_import_version` (`lifecycle.rs:73-84`).

Nothing owes the pair again; the only writers of `pairs_owed` are `writers.rs:268`, `lifecycle.rs:244` and
`convert.rs:200`. So the pair is never made.

**The fix: a pair is a fact about two stored analyses.** Every analysis change goes through REPLACE, which deletes the
pairs (`writers.rs:309`, `354`) and owes them again (`writers.rs:340`).

- **The stored side** is every analysis with hashes and `pdq_quality ≥ MIN`, except displaced rows.
  - Pairs with missing files stay hidden by `present_pdq_pairs` (`schema.rs:722`).
  - The duplicates filter already requires the partner to be present (`query/src/lib.rs:75-82`).
- **Displaced rows** (`volume IS NULL`) are excluded. `displace` keeps the analysis but clears the identity
  (`writers.rs:107`). The only way back is `replace`, which deletes the analysis first (`pair.rs:435-438`). So their
  pairs could never be shown, and a new row for the same picture would pair with its own tombstone.
- **The queries keep `OWED`'s definition** (`work.rs:136`): owed, present and not pending. An owed picture that is
  missing or pending is paired by the first pass after it returns. That keeps all of these in agreement:
  - `has_owed_pairs` and `owed_pairs`;
  - the pass gate (`pairs.rs:109`) and the progress total (`pairs.rs:128`);
  - the CLI status (`cli/src/lib.rs:346-348`);
  - `NOT_DUPLICATES` (`query/src/lib.rs:356-357`).
- **The legacy count** covers present, non-pending files only. `gallery import --again` cannot reach the others
  (`work.rs:212`, `sql.rs:132`).
- **Before the change,** count on the scratch copy the files it affects and the pairs it adds.

**Tests:**

- end to end, each case leaves the pair visible with no new owed work:
  - X is paired while Y is missing, then Y comes back through the upsert, a tier-1 rebind and a tier-2 rebind;
  - a rotation is done and undone around a pass;
  - an mtime reverts;
  - `import_version` flips;
- `OWED`, `has_owed_pairs`, `NOT_DUPLICATES` and the progress total agree with the pass's queries;
- the legacy count leaves out missing files;
- `writers.rs:1332-1352`, which asserts the old exclusion, is rewritten.

## Step 7: the write

**Status: done, kept** (the guard and the epoch, 6 pairs against step 6: write −52 to −84% on every workload; pass
−3.96% on all owed at 1 worker, −14.98% at 12, −9.07% on 3,000 owed at 12, the rest level; `PROFILING.md`, "The
write"). Item 1 found the guard's reads over half the write (~190 of ~340 ms), and the set-based guard alone saves 7%,
so the epoch was built (migration 14). Departures:

- **No trigger on `pairs_owed`.** The guard reads only analyses, so an equal epoch needs only "no analysis changed"; a
  row owed again since the load is marked done by a write whose pairs were made from the analysis it still holds,
  guarded or not (the old per-row guard did the same), and another build's pairing is `pairing_version`'s check. A
  per-row trigger there doubled `owe_pairs_again` (137 → 280 ms on 187,804 pictures). `set_import_version` and the
  scan's new-file path write `files` only, which no epoch trigger watches.
- **The analysis triggers:** every INSERT (a REPLACE's own delete fires no trigger), a DELETE of a row with hashes
  (a file's cascade reaches it, tested), and an UPDATE changing `file_id`, `pdq_dihedral` or `pdq_quality`.
- **The pairing version** is also checked after the load: a pass whose catalog is another build's pairing writes nothing.
- **The statements** are prepared through the connection's cache, once per pass.
- **`WRITE`** (item 4) is left at 2,000 pictures: a batch now holds the lock less than half as long (121 ms over 94
  batches against 291). Step 9 tunes it within that bound.
- **The import flush** (`ir::catalog::import_flush::batch`, 500 stores): +2.20% instructions, about 6,400 a store
  for the insert trigger. The pass gains and the import loses, so the two numbers went to you, as this step says: kept.
- **The single copy of the hashes:** done, kept against `57bda80` on every workload; peak commit −23 MiB on all owed
  (202 - 203 → 179 - 180) and on 30,000 (137 - 138 → 114). `Joiner::original` undoes the store's permutation
  (`unpermuted`, tested to round-trip every single-bit hash and 2,000 random ones). `Settings::guard`
  (`PG_PROBE_GUARD`) forces every write through the guard, so the pass-level test and the oracle run that path on a
  catalog nothing changes. `transpose_rows` is `#[inline(always)]` (its second caller made LLVM stop inlining it into
  `permuted`: +14 ms a join), and the load's copy is freed on a thread of its own (1.9 ms on the pass's thread).

1. **Find where the 243 ms goes, first:**
   - statement overhead;
   - `analysis` seeks;
   - the `pairs_owed` delete;
   - WAL bytes and checkpoints;
   - whether the 35 ms commit includes an auto-checkpoint.
2. **A set-based guard replaces today's per-row guard.** It needs no migration.
   - **The read.** Inside the write transaction, after the lock (`transaction.rs:14`), it reads the batch's owners and
     partners from `analysis`, deduplicated, by exact ids.
     - The ids are bound as one array through rusqlite's `array` feature (`rarray`, added at `Cargo.toml:39`), and
       the statement is prepared once per pass.
     - Not an IN-list, whose changing length forces a re-prepare per batch.
     - Not `json_each`, which parses text.
     - Not a range, because batches are sparse (`pairs.rs:119`, `183`).
   - **A picture passes** when its row came back, its blob is not NULL, the blob equals `to_blob(original(i))`, and its
     quality is unchanged.
     - A missing row means a rebind deleted it (`pair.rs:353`). Inserting the pair anyway would fail the foreign key
       and abort the batch.
     - A NULL blob is a failed re-analysis (`writers.rs:270-272`).
     - No `files` column is read. After step 6, presence and pending-ness no longer decide whether a pair is valid.
   - **The writes.**
     - A pair is inserted when both of its pictures pass, whatever its owner's result, with
       `ON CONFLICT … DO UPDATE`.
     - One array-bound `DELETE FROM pairs_owed` removes the passing owners. It is never a range, because an owed row
       can belong to an ineligible analysis (`writers.rs:1204`).
   - **The algorithm version.** Each write transaction reads `meta.pairing_version`, and the pass stops if it is not
     this build's (`lifecycle.rs:235-247`).
3. **An epoch, only if it pays.** Measure the guard's re-read against the same write without it.
   - The epoch is built only if skipping the read is a real gain, and its triggers pass the keep rule on:
     - the import flush bench;
     - `owe_pairs_again`;
     - `set_import_version`;
     - the scan's new-file path.

     If the pass gains while the import loses, the two numbers come to you.
   - **If built** (migration TO_14), `meta.pairing_epoch` is bumped by triggers with `WHEN` clauses:
     - on `analysis`: INSERT, DELETE, and UPDATE OF `pdq_dihedral` and `pdq_quality`;
     - on `pairs_owed`: INSERT only.

     A `pairs_owed` DELETE does not bump: its only deleters are `writers.rs:412`, `convert.rs:208` and cascades,
     which the `analysis` triggers cover.
   - **Rebuilds and tests.** A rebuild recreates the triggers (`rebuild.rs:189`), and they are listed in
     `TRIGGER_TESTS`.
   - **The write.** When the epoch equals the load's, the batch skips the read. Once it differs, that batch and every
     later one take the guard.
4. **`WRITE` is bounded by write-lock hold time,** as the `watch_holds` fixture measures it. No batch holds the lock
   longer than today's 2,000-picture batch, and a stop lands no later.

**Tests:**

- A differential test against today's `PairsWriter`, kept as the oracle, under random changes between the load and the
  write, including:
  - an owner replaced by a failure;
  - an analysis deleted;
  - a rebind (no foreign-key error, the pair skipped);
  - a re-analysis with identical hashes;
  - a partner that is also an owner;
  - repeated partners;
  - a partner re-analysed;
  - `store_failure` on a partner;
  - missing and pending owners and partners;
  - a malformed owed row inside the batch's id range.
- The `pairing_version` check.
- TO_14, if built:
  - each trigger's `WHEN`;
  - REPLACE and cascades;
  - a rebuild;
  - `TRIGGER_TESTS`.

## Step 8: the load

**Status: done, kept** (the lookup-free load, 6 pairs against `459c8e0`: load −19 to −35% on every workload, real
gains; pass −1.55% on all owed at 1 worker, −10 to −23% on a few owed; `PROFILING.md`, "The load"). Item 1 found the
lookups ~85 ms of the 212 ms load and fetching pages about half of every statement. Departures:

- **The id sets are sorted lists, not bitmaps.** A bitmap sized by `max(id)` has no bound (one large id), and the
  tests' sparse and large ids are what this item asked for; a sorted list's walk costs one step a member.
- **The files with an identity, not the displaced ids,** are read (21.6 ms against ~0): "an analysis with no `files`
  row stays excluded" needs the set of files that exist. The foreign key's cascade leaves no such row, so this costs
  ~20 ms for a row the schema cannot hold; it is kept, as the item says.
- **`count(*)` goes:** the identity set's size bounds the columns.
- **Mixed snapshots and the epoch** (item 3): a split load is safe only if the marks are read in a snapshot no later
  than every part's, so a change after them moves the epoch and the writes guard; the reader reads them first.
- **The split load (item 3): done, kept** against `bed9346` (pass −8.3% on all owed at 12 workers, −30.0% on 300 owed,
  −21.8% on 3,000; load −44 to −49% at 12; one worker level). It needed SQLite's memory statistics off (`bed9346`):
  their global mutex serialized the parts' connections (an open took 28 - 37 ms with 12 at once, 2 ms alone). The
  parts are cut at equal counts of the identity set, which the reader reads first, and the other sets are read beside
  them; the parts fill slices of the columns, moved together after (one copy of the hashes). One part (one worker, in
  memory) is the lookup-free load as it was. The parts' count is the join's threads: from 3 parts the wall is the
  reader's own reads (87 - 98 ms) and more parts add cycles (+40% on 300 owed at 12), which step 9 tunes.

1. **Time its parts first,** with cycle samples on the load's thread.
2. **No per-row lookups:** one connection, one read transaction (`work.rs:100`).
   - From `files`, read only what step 6 needs, through the narrowest covering index `EXPLAIN QUERY PLAN` confirms:
     - the displaced ids;
     - the query-eligible ids: `files_present` (`schema.rs:638`) minus `files_to_import`, which is not in id order
       (`schema.rs:640`), as a bitmap sized by `max(id)`.
   - Scan `analysis` with `pdq_quality >= ?1` in SQL, and merge `pairs_owed` in rowid order.
   - An analysis row with no `files` row stays excluded, as today's inner join does.
   - Keep the malformed-blob error (`work.rs:116`).
3. **A parallel load, as a variant.**
   - The sets are read once.
   - `T` connections scan `analysis` id ranges, claimed with `fetch_add` over equal-count boundaries, and share one
     cache budget.
   - The ranges are concatenated in id order.
   - Mixed snapshots are safe, because the guard checks every hash at write time.
   - An in-memory catalog reads on its one connection (`core.rs:215`).
4. **If the load is mostly SQLite's own page reads** after this, that is recorded and the load is left there.

**Tests:** the lookup-free load equals a reference: today's query with step 6's eligibility. The cases:

- missing and pending legacy rows;
- owed rows of ineligible analyses;
- displaced rows;
- sparse and large ids;
- a malformed blob.

## Step 9: re-tune what the changes made stale

**Status: in progress.** The join keeps every logical core (6 workers lose 21 - 23%); the load keeps one part a
join thread (fewer than 6 loses time, 6 gains none). Fitting the few-owed crossover found `Joiner::new`'s permute
on one thread (50 - 66 ms at every count, the plan's step 1 item left undone): now on the join's threads,
-37% on the join for 300 owed at 12 workers. The few-owed threshold is now the line through the measured crossovers, 221 + 38 owed pictures a thread
(`FEW_BASE`, `FEW_PER_THREAD`): the old 125 a thread was 2.3x too high at 12 threads (joins −17 to −31% where the
two rules differ). The cache stays at 16 MB (2 - 8 MB gain nothing, and 2 MB slows the split load). `WRITE` is 4,000,
the most within step 7's lock-hold bound (commit −57 to −67%). `BLOCK` stays at 128 (32, 64 and 256 gain nothing; the
starved time, ~5%, is the stages' tails), now a run-time `Tuning::block`. Settings switches: `PG_PROBE_LOAD_PARTS`, `PG_PROBE_WRITE`; the
crossover sweep is `tools/profiling/pairing/few_sweep.ps1`.

Each setting takes the simplest value within +1% of the best, with peak commit recorded:

- the few-owed threshold (`FEW_ENTRIES_PER_THREAD`);
- `SLICE`, on the synthetic 500,000-picture corpus;
- `CACHE_KIB`, now including 2, 4 and 8 MB, since only 16 MB and up was measured (`pairs.rs:31-32`);
- `WRITE`, within step 7's bound;
- the thread count: logical against physical cores;
- `BLOCK`, the scan's buckets a task, and the sort's tasks per phase: since step 5 a phase's tail is the team's
  only wait, and on four slices of 50,000 at 12 workers the threads wait 0.8 - 1.8 s of their time (`starved`, about
  70 - 150 ms of each pass's joins by the clock).

## Step 10: docs and benches

- **`PROFILING.md`:** each result with its interval.
- **P-13:** updated or closed.
- **`PAIRING_PASS_PLAN.md`:**
  - the sorter rule (G3) is replaced by the pacing rule;
  - the 330 MB target is removed (`:14`, `:64`, `:246`, `:261-264`), and `PROFILING.md:713` is reworded to match;
  - the callgrind recipe (`PROFILING.md:719`, `742`) drops `PG_PROBE_SORTERS`.
- **`crates/README.md`** and **`cli.ftl`.**
- **The `pairing.join` bench.**
- **The replay and keep-rule scripts** in `tools/profiling/pairing/`.

## Not doing

| Idea | Why not |
|---|---|
| Fewer stored orientations | Loses 60 or 33 of the 5,277 pairs |
| Memory-mapped reads | Ruled out: a read error would crash the app |
| Fixed trip counts, the swapped loop, small-bucket special cases | Measured: 1.90x the iterations, the same counts, 1.00x |
| AVX-512 VPOPCNTQ | Not on this CPU |
| BMI2 | Not needed (step 3's mask) |
| Other chunk counts | Measured worse |
| A sorted index kept between passes | A second source of truth |
| `PRAGMA data_version` as the change signal | Changes on every unrelated commit |
| Epoch bumps from each writer | The writers are scattered; one would be missed |
| Splitting sort regions | Several tasks would write one buffer; the tail is already small |
| Guided self-scheduling | Claims would stay at the latency cap until the last few |
| A trigger re-owing on `missing` 1 → 0 | Misses the pending cases of step 6 |

## Verification, end to end

- **Per crate.** `cargo test` for `pg-dedup`, `pg-catalog`, `pg-import` and `pg-library`, plus clippy with the
  workspace lints. A step is not done until its tests above and the existing brute-force tests pass: every owed share,
  every kernel, every thread count, both paths.
- **Real data,** on scratch copies:
  - `oracle_d.ps1` on all seven paths: identical pairs, and after step 6, only the listed additions;
  - the keep rule per step;
  - cycle samples by region, and the mispredict count before and after step 3;
  - peak commit recorded;
  - TO_14, if built, applied to a copy of a v13 catalog.
