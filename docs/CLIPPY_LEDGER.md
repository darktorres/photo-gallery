# Clippy ledger: rules to enable

Tracks which clippy lints the workspace should enable, the code that each one would flag, and where each change stands.
Baseline recorded 2026-11-05 at commit `c988b9c`.

## Current configuration

- `Cargo.toml` `[workspace.lints.clippy]` enables `undocumented_unsafe_blocks` and the Phase 1 lints: `dbg_macro`,
  `todo`, `unimplemented`, `mem_forget`, `exit` and `panic` at deny; `allow_attributes` and `string_slice` at warn. The
  Phase 2 lints `print_stdout` and `print_stderr` are at deny, the Phase 3 lints `unwrap_used`, `expect_used` and
  `indexing_slicing` too, and the Phase 4 lints (`cast_lossless`, `cast_possible_truncation`, `cast_sign_loss`,
  `cast_possible_wrap`, `cast_precision_loss`, `as_conversions`) too.
- Each crate opts in with `[lints] workspace = true`. The vendored crates in `vendor/` are outside the workspace.
- `clippy.toml` bans `std::collections::HashMap`, `HashSet` and `RandomState`.
- `just lint` runs `cargo clippy --workspace --all-targets -- -D warnings`, so a lint at `warn` fails the gate.

## Baseline counts

Unique diagnostics from one run over the library and binary targets, with `#[cfg(test)]` code excluded. The run enabled
the candidate lints below on the command line (`-W`), so the counts are for those lints alone, not for the workspace as
configured.

| Lint | Hits | Group |
|---|---|---|
| `as_conversions` | 376 | restriction |
| `indexing_slicing` | 264 | restriction |
| `unwrap_used` | 135 | restriction |
| `expect_used` | 31 | restriction |
| `cast_possible_truncation` | 159 | pedantic |
| `cast_possible_wrap` | 71 | pedantic |
| `cast_sign_loss` | 52 | pedantic |
| `cast_precision_loss` | 48 | pedantic |
| `cast_lossless` | 35 | pedantic |
| `string_slice` | 16 | restriction |
| `print_stdout` | 29 | restriction |
| `print_stderr` | 29 | restriction |
| `panic` | 3 | restriction |
| `allow_attributes` | 5 | restriction |
| `dbg_macro`, `todo`, `unimplemented`, `mem_forget`, `exit` | 0 to 1 each | restriction |
| `undocumented_unsafe_blocks` | 0 | restriction |

Style and documentation lints, not recommended: `use_self` (424), `must_use_candidate` (338), `missing_errors_doc` (190),
`too_long_first_doc_paragraph` (128), `missing_const_for_fn` (110), `doc_markdown` (87), `missing_panics_doc` (74).

Cheap style wins: `derive_partial_eq_without_eq` (10), `redundant_closure_for_method_calls` (16), `float_cmp` (7),
`semicolon_if_nothing_returned` (4), `needless_pass_by_value` (4), `match_same_arms` (5).

## Phase 1: enable the lints with few or no hits

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `dbg_macro` | deny | 0 | none | done |
| `todo` | deny | 0 | none | done |
| `unimplemented` | deny | 0 | none | done |
| `mem_forget` | deny | 0 | none | done |
| `exit` | deny | 1 | `expect` at the site | done |
| `panic` | deny | 3 (build script), 12 (test code) | `expect` at the site; `allow-panic-in-tests` for `#[test]` functions | done |
| `allow_attributes` | warn | 5 (+1 Linux-only) | `allow` to `expect` with a reason; one `allow` was unneeded and removed | done |
| `string_slice` | warn | 16 (+5 in tests) | `get` where the function already returns `Option`; `expect` where the cut is by construction | done |

Results of the change (committed as `8be49f3`):

- The `exit` hit is `fatal` in `crates/desktop`, which exits after the message box by design. The two `exit` calls in `crates/gallery/src/main.rs` are not flagged.
- The three `panic` hits are in `crates/video/build.rs`. A build script fails by panicking, so the file has one `#![expect]`.
- `allow-panic-in-tests` (root `clippy.toml`) suppresses `panic` in `#[test]` functions, checked with a probe in clippy 1.99.1.
  It does not cover every helper: `dir.rs`'s `refused_for` and the integration-test helper `unescape` still need an `expect`. An
  `expect` that is not needed is reported as unfulfilled, so each one is checked.
- `crates/client` and `crates/model` have their own `clippy.toml`. clippy reads that file instead of the root one, not merged, so
  those two crates had neither the root's hash-collection bans nor `allow-panic-in-tests`. Each now repeats the root's settings.
  Neither crate uses a hash collection, so the bans cost nothing.
- The `string_slice` hits: `date.rs` now uses `get`, since its function already returns `Option`. `path.rs` and `metadata/lib.rs`
  keep their indexing under an `expect`, because the boundary is guaranteed by construction. `model/tree.rs` (three functions) and
  `yandex/lib.rs` (two) cut between whole pushed parts or `find` results, so each carries an `#[expect]` that says why. Five more
  are in test helpers.
- The baseline ran on library and binary targets. `just lint` runs `--all-targets`, which found the test-code hits above.
- `crates/render/examples/decodediff.rs` (untracked, a throwaway harness) did not compile in this checkout, so the gate
  was run with `--lib --bins --tests --benches` while it existed. The file is deleted now; `--all-targets` has not been
  re-run since.
- The Linux-only `expect` in `crates/platform/src/dir.rs` is not seen by clippy on Windows. It is checked in WSL, on the working
  tree, not on committed code.

## Phase 2: decide the print lints

| Lint | Hits | Level | Status |
|---|---|---|---|
| `print_stdout` | 29 | deny | done (`520086a`) |
| `print_stderr` | 29 | deny | done (`520086a`) |

Decided: printing is for the entry points and the tools. Each place that prints allows it where it prints, with an
`#[expect]` that gives the reason:

| Where | Prints | Allowed by |
|---|---|---|
| `pg-cli` | the command line's reports (stdout) and progress and errors (stderr) | `#![expect]` at the crate root |
| `gallery` `main.rs` | the CLI's error, and the notice of a build without the window (stderr) | `#![expect]` at the crate root |
| `pg-testkit` | a skipped test's notice (stderr) | `#[expect]` on `missing_tool` |
| `bench` (non-Linux stub), `analysis` example | their output | `#[expect]` on the stub's `main`; `#![expect]` on the example |
| `pg-desktop` `PG_TRACE=1` timings | stderr, on purpose | `#[expect]` on `trace::install` |
| tests | test output | `allow-print-in-tests` in `clippy.toml` |

Build scripts need nothing: clippy does not lint the `println!` directives that cargo reads from their stdout.

The app's errors are not printed at all. `pg-desktop` had 21 `eprintln!` calls for errors and notes. A GUI build shows
stderr only when started from a terminal, so most went nowhere. They now go through `panic_log::note`, which appends to
`gallery.log` beside the catalog (the app's existing channel for this). When the log can't be written, or before it is
installed, the line goes to stderr, so no message is dropped. `pg-trace` was the first candidate, but it is for
the startup stages, not for errors.

Results of the change (uncommitted):

- The gate passes: `cargo clippy --workspace --all-targets --keep-going -- -D warnings`, with no unfulfilled expects.
  That includes the test prints, so `allow-print-in-tests` works for `#[test]` and `#[ignore]` functions.
- Behaviour change: the desktop's errors, and the notice that `--yandex-fixture` answers from a recorded session, are
  in `gallery.log` rather than on stderr. A terminal sees them only when the log can't be written.
- `pg-exiftool`'s "ExifTool not found: skipped" is in a `#[test]`, so it is covered by `allow-print-in-tests`; no library
  prints.

## Phase 3: panic-freedom

| Lint | Hits (production code) | Notes | Status |
|---|---|---|---|
| `unwrap_used` | 136 | Fix at each site; `expect` with a reason only for a true invariant | done |
| `expect_used` | 31 | Same | done |
| `indexing_slicing` | 310 | Fix at each site; no module-level allow | done |

Hits counted from `cargo clippy --workspace --lib --bins` with the three lints at warn: 477 unique sites, in 25 crates
(`analysis` 69, `model` 53, `edit` 52, `jobs` 38, `catalog` 34, `client` 32, `dedup` 28, `container` 24, `metadata` 24,
`codec` 23, `app` 19, `library` 14, `testkit` 12, `desktop` 10, and fewer elsewhere). Test code is excluded by
`allow-*-in-tests` in `clippy.toml`.

Results (uncommitted, pending review of the behaviour changes below): the three lints are `deny` in `Cargo.toml`. The gate
passes on Windows (`cargo clippy --workspace --all-targets -- -D warnings`), and in WSL Ubuntu with the Linux CI job's
commands: the same clippy gate with `--exclude pg-desktop --exclude pg-gallery`, then `cargo test` on the same set.

The `expect`s that remain for these lints: `pg-desktop`'s `app::get` (`expect_used`: the app is opened before Qt starts),
and `pg-bench`'s `ir/linux.rs` `or_stop` (`panic`: a benchmark whose setup or measured call fails stops the run with the
error, since it has nothing to measure). The other `panic` expects are Phase 1's (`video/build.rs`, `platform/dir.rs`).

Rules for each site, in order of preference:

1. A lock: `lock().unwrap_or_else(PoisonError::into_inner)` when the guarded state is a plain value that stays
   consistent if a holder panicked. A panic would otherwise take every later caller down with it.
2. A fallible call or parse: propagate with `?` into the crate's error type (or `anyhow`, where the crate uses it), or
   turn an absent value into the error the caller already reports (`ok_or`, `let else`).
3. Indexing: iterators, `chunks_exact`, `split_first`, `windows`, slice patterns, or `get` with the failure handled as
   in (2). Where the bounds come from a length check in the same function, let the check narrow the type (a fixed-size
   array, `try_into`) so no index is needed.
4. `expect` with a `reason` (`#[expect(clippy::expect_used, reason = "...")]` on the statement) only when the invariant
   holds for every input and no type can carry it. Each reason says what the invariant is.

Not allowed: `#[allow]`, module-level or crate-level allows, and `get_unchecked`.

A `pub` signature that other crates call is changed, and every caller in the workspace is updated with it (`Result` or
`Option` in place of the panic). The first round left these for a decision; the owner has decided to make the change.

Behaviour: valid inputs give the same results as before. An input that used to panic now returns an error or the
caller's existing "absent" value; each such change is listed in the commit message.

## Phase 4: lossy casts

These match the defect class of the type-mismatch findings (the `lossy-cast` category).

| Lint | Hits | Notes | Status |
|---|---|---|---|
| `cast_lossless` | 35 (55 across all targets) | Mechanical: `From` instead of `as`; `cargo clippy --fix` made all 55 | done |
| `cast_possible_truncation` | 159 (340 across all targets) | Phase 4 policy below | done |
| `cast_sign_loss` | 37 (37 across all targets; WSL adds none) | Sign-loss policy below | done |
| `cast_possible_wrap` | 47 (45 on Windows, 2 Linux-only: `platform/clock.rs`, `platform/file.rs`) | Wrap policy below | done |
| `cast_precision_loss` | 99 (99 across all targets; WSL adds none) | Precision policy below | done |
| `as_conversions` | 226 (226 across all targets; WSL reports 196, adds none) | As-conversions policy below | done |

### Truncation policy (`cast_possible_truncation`)

Rules for each site, in order of preference:

1. Integer to a narrower integer: `T::try_from(x)`, with the error propagated by `?` into the function's error type
   (or `anyhow`), where the caller can report it.
2. The same, where the function cannot fail and the value is bounded in the same expression (a `clamp`, `min`, `%` or
   mask): `T::try_from(x).unwrap_or(T::MAX)`. It saturates, and for every in-range value it gives the same result.
3. Float to integer: Rust's `as` saturates, but the lint still flags it. Each target type gets one helper that clamps
   and converts, with one `#[expect(clippy::cast_possible_truncation, reason = "...")]` on it, and the call sites use
   the helper. The reason says what the clamp guarantees. A helper is the only place a float cast is written.
4. An `expect` on a statement is allowed only where neither a type nor a helper can carry the bound (rule 4 of the
   Phase 3 policy). Its reason says what the bound is.

Not allowed: `#[allow]`, module-level or crate-level allows, and disabling the lint in `clippy.toml`. Tests are linted
too (the gate runs `--all-targets`), so test code follows the same rules; `allow-unwrap-in-tests` covers `unwrap` on a
`try_from` there.

Behaviour: valid inputs give the same results. An out-of-range value used to wrap (or saturate, for a float); it now
errors (rule 1) or saturates (rule 2). Each such change is listed in the commit message.

Results: the lint is `deny` in `Cargo.toml`. The 340 sites are fixed with 19 `expect`s, all on float helpers (rule 3),
none on statements. The Windows gate passes, and so do the Linux clippy gate and the Linux tests (WSL). Linux-only code
(`platform/src/clock.rs`'s non-Windows `local_time`) was the only code the Windows gate missed.

### Sign-loss policy (`cast_sign_loss`)

Rules for each site, in order of preference:

1. Signed integer to unsigned integer, for an index or a size: `usize::try_from(x)`. A failure takes the path the code
   already takes for a miss (`ok()` then `get`, `let else`), or it is propagated with `?` where the caller reports
   errors. A negative index always missed (it wrapped to a huge index), so the miss path keeps every result.
2. Bounded by a `clamp` or `max` on the value itself, in the same expression or in the binding just before it:
   `T::try_from(x).unwrap_or(floor)`, where `floor` is the bound's lower end. The fallback is never reached, so the
   result is the same for every value the code can see.
3. Float to unsigned integer: the truncation helper for that target type (rule 3 of the truncation policy) carries this
   lint too. Its `#[expect]` gets `clippy::cast_sign_loss`, and its reason says the clamp's lower bound makes the value
   non-negative. No new helper.
4. A value whose bits are the point (a file id stored as `i64`, or a test that round-trips a `u64` through the catalog):
   `cast_unsigned()`. It keeps the same bits and is not a sign check; `try_from` would fail on every id above
   `i64::MAX`, which is the wrong answer.
5. A statement-level `expect`: only where none of the above fits. Its reason says what holds.

Not allowed: `#[allow]`, module-level or crate-level allows, and disabling the lint in `clippy.toml`. Test code follows
the same rules; `allow-unwrap-in-tests` covers `unwrap` on a `try_from` there.

Behaviour: valid inputs give the same results. A negative value that used to wrap now takes the miss path (rule 1) or
the floor (rule 2). Each such change is listed in the commit message.

Results: the lint is `deny` in `Cargo.toml`. The 37 sites are fixed: 12 float helpers gain `cast_sign_loss` in their
`expect` (rule 3), 25 integer sites use rules 1, 2 and 4, and no statement-level `expect` was needed. The Windows gate
passes, and so do the Linux clippy gate and the Linux tests (WSL, 397 tests). The Linux run found no site that the
Windows run missed; the desktop crates and `platform/src/dir.rs` are Windows-only and were checked on Windows.

### Wrap policy (`cast_possible_wrap`)

Rules for each site, in order of preference:

1. A bit pattern: a `u64` id, volume or hash that is stored as the `i64` SQLite holds, or a Windows `DWORD` read as an
   `i32`. The bits are the value, so the conversion is `cast_signed()` (and `cast_unsigned()` for the reverse). It is not
   a check: `try_from` would reject every id above `i64::MAX`, which is the wrong answer.
2. A value that can be out of range: `T::try_from(x)`, with the failure propagated by `?` into the function's error type,
   or taken on the path the code already takes for an absent value.
3. A length, count or size that cannot reach the target's maximum: `T::try_from(x).unwrap_or(T::MAX)`. A slice or `Vec`
   length is at most `isize::MAX`, which `i64` holds. A Qt `int` saturates at `i32::MAX`, which no real image dimension
   or row count reaches. A platform time (FILETIME, Unix seconds) saturates at `i64::MAX`, a date past year 30000. The
   comment at the site names the bound when it is not obvious. Saturating is wrong where the value is a counter that must
   keep changing (a version): that one is rule 1, with `cast_signed()`, which keeps the bits the code already compares.
4. Test code follows the same rules; `try_from(..).unwrap()` is allowed there (`allow-unwrap-in-tests`).
5. A statement-level `expect`: only where none of the above fits. Its reason says what holds.

Not allowed: `#[allow]`, module-level or crate-level allows, and disabling the lint in `clippy.toml`.

Behaviour: valid inputs give the same results. An out-of-range value used to wrap; it now errors (rule 2) or saturates
(rule 3). Each such change is listed in the commit message.

Results: the lint is `deny` in `Cargo.toml`. The 47 sites are fixed with no `expect`s: catalog ids, BMP header fields,
the Windows error code and the version counter by rule 1; schema and file lengths, file sizes and times by rules 2 and 3;
test code by `try_from(..).unwrap()`. The Windows gate passes, and so do the Linux clippy gate and the Linux tests
(WSL, 397 tests). The two Linux-only sites are in `platform/clock.rs` and `platform/file.rs`.

### Precision policy (`cast_precision_loss`)

Rules for each site, in order of preference:

1. A narrower integer converts exactly with `From`: `f64::from` for `u8`, `u16`, `u32`, `i8`, `i16` and `i32`, and
   `f32::from` for `u8`, `u16`, `i8` and `i16`. `cast_lossless` already asks for these, so the sites left for this lint
   are the wide types below, and a small loop index in `f32` is rule 1.
2. A `usize`, `u64` or `i64` count, size or timestamp: a helper in `pg_model::num` (`count`, `size` and `signed`), one
   per source type, each with one `#[expect(clippy::cast_precision_loss, reason = "...")]`. The reason is the bound: an
   `f64` holds integers exactly up to 2^53 (9 PB of bytes, 9 quadrillion items), and no count, size or timestamp the app
   holds reaches it.
3. A crate that does not depend on `pg-model` (`jobs`) keeps one local helper with the same expect and the same reason.
4. A statement-level `expect` only where the bound holds by construction and no type or helper can carry it (a 24-bit
   value in an `f32`, for example). Its reason says the bound.

Not allowed: `#[allow]`, module-level or crate-level allows, and disabling the lint in `clippy.toml`. Test code follows
the same rules.

Behaviour: no change. Each helper is the same `as` conversion, so the rounding is the same for every input.

Results: the lint is `deny` in `Cargo.toml`. 94 sites call the helpers: `pg_model::num::count` for `usize`, `size` for
`u64` and `signed` for `i64`, and `jobs` calls its one local helper, which takes `u64` (its counters are `u64`). Four
helpers and the local one carry the expect, each with its bound. The five `f32` sites are fixed without a helper: the
two in PDQ test code go through `u16::try_from(..).unwrap()` and `f32::from`, the dither loop's `k` is a `u8` and takes
`f32::from(k)`, and the two casts on the 24-bit value in `codec/codecs.rs` become one statement-level expect (rule 4),
since the division by `16_777_216.0` is the same arithmetic. The Windows and Linux clippy gates pass, and so do the
Linux tests.

### As-conversions policy (`as_conversions`)

The lint flags every `as` expression, so each one is replaced by the form that states its domain. Rules, in order of
preference:

1. Lossless by `From`, `into` or `try_from` where the target always holds the source: `usize::from(u8)`,
   `usize::from(u16)`, `usize::from(bool)`, `u64::from(u32)`, `i64::from(i32)`, `u32::from(char)`, `char::from(u8)`,
   `f64::from(f32)`. In a `const`, where these are not `const fn`, the value is a typed literal with the character in a
   comment (`0x5C` for `\`), or the item is a `fn`.
2. A value that can be out of range for the target: `T::try_from(x)`, propagated with `?` or taken on the path the code
   already takes for an absent value, as in the truncation, sign and wrap policies.
3. An index, length, count or dimension between `usize` and a fixed-width integer: `usize` from `u32` is
   `pg_model::num::index`, which saturates at `usize::MAX`. Every target has `usize` of at least 32 bits, so the
   saturation never fires; a compile-time check in `num` asserts that. `u64`, `i64` and `u32` from `usize` are
   `try_from(..).unwrap_or(MAX)` (saturating), as in the wrap policy.
4. Float to integer: one helper per target type in `pg_model::num` (`f64_to_u8`, `f64_to_u16`, `f64_to_u32`,
   `f64_to_u64`, `f64_to_usize`, `f64_to_i32`, `f64_to_i64`, `f64_to_f32`). The caller clamps to the target's range (or rounds and
   clamps) as it does now; the helper is the one `as` for that target type, with one expect that names the clamp and that
   NaN is 0. An `f32` source widens with `f64::from` first. The local helpers (`clamp_u8`, `f32_to_u8`, `round_u8`,
   `round_u32`, `round_u64`, `round_i64`, `round_i32`, `to_i32` and the inline clamps in `pdq`, `screenshot`) call these.
5. Pointers: an address is `ptr.addr()`, a pointer cast is `.cast()`, and a reference becomes a pointer with
   `ptr::from_mut` or `ptr::from_ref`. A raw pointer from an integer is the FFI type's own constructor when it has one.
6. Enums and bytes: a discriminant or byte from a `char` or enum is a `match` or a `const fn` that returns the value,
   not an `as`.
7. Test code follows the same rules; `try_from(..).unwrap()` is allowed there.
8. A statement-level `expect` only where no rule above fits. Its reason states the bound.

Not allowed: `#[allow]`, module-level or crate-level allows, and disabling the lint in `clippy.toml`.

Behaviour: valid inputs give the same results. Float helpers are the same `as` after the same clamp, so rounding and
NaN handling do not change. A value that used to wrap through an integer `as` now errors or saturates (rules 2 and 3),
and each such change is listed in the commit message.

Results: the lint is `deny` in `Cargo.toml`. All 226 sites are converted with no call-site allows or expects. `pg_model::num`
holds `index` (u32 to usize), the float helpers `f64_to_u8` through `f64_to_f32` (one expect each, naming the clamp), and
the three precision helpers. The local float helpers (`clamp_u8`, `f32_to_u8`, `f32_to_u16`, `round_u8`, `round_u32`,
`round_u64`, `round_i64`, `round_i32`) now call those helpers and keep their clamps, and their own expects are removed.
`PathRules::sep_byte` is a new `const fn` returning the separator as a byte, used where `sep() as u8` was. Windows
and Linux clippy gates pass with the lint denied, and the Linux tests pass (WSL, 397 tests).

Behaviour: valid inputs give the same results. The fallbacks of the `try_from(..).unwrap_or(..)` forms are unreachable on
64-bit targets. On 32-bit, the WebP chunk length in `container` used to overflow in `u32 as usize + 8` (a panic in
debug); it now saturates and is clamped to the buffer's length. The writeback sleep and the file length in `writeback`
propagate a `try_from` error with `?`, where a wrap used to happen; neither constant can reach it.

Phases 5 to 14 group the remaining lints by the work each one needs: enable now, correct, decide, or leave off. Phases 1 to 4 are done.

## Phase 5: enable lints with no hits

To be enabled in one change. Each lint has zero hits in the current tree, so the change is configuration only. Rustdoc lints (`rustdoc::all`) also have zero hits on Windows and WSL.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `allow_attributes_without_reason` | warn | 0 | none | not started |
| `try_err` | deny | 0 | none | not started |
| `get_unwrap` | deny | 0 | none | not started |
| `unwrap_in_result` | deny | 0 | none | not started |
| `string_to_string` | warn | 0 | none | not started |
| `rest_pat_in_fully_bound_structs` | warn | 0 | none | not started |
| `unneeded_field_pattern` | warn | 0 | none | not started |
| `unsafe_op_in_unsafe_fn` | deny | 0 | none | not started |
| `single_use_lifetimes` | warn | 0 | none | not started |
| `redundant_lifetimes` | warn | 0 | none | not started |
| `unnameable_types` | warn | 0 | none | not started |
| `impl_trait_redundant_captures` | warn | 0 | none | not started |
| `trivial_casts` | warn | 0 | none | not started |
| `unused_import_braces` | warn | 0 | none | not started |
| `unused_lifetimes` | warn | 0 | none | not started |
| `macro_use_extern_crate` | warn | 0 | none | not started |
| `meta_variable_misuse` | warn | 0 | none | not started |
| `unused_macro_rules` | warn | 0 | none | not started |
| `non_ascii_idents` | warn | 0 | none | not started |
| clippy cargo group, except `multiple_crate_versions` (Phase 12) | warn | 0 | none | not started |
| the 171 names below | warn | 0 | Add each name at `warn`; no code change expected | not started |

These 171 lints are not in the default set, have zero hits in the current tree on Windows and WSL, and are not named
elsewhere in this ledger. Enabling them is configuration only: each has zero sites, so the gate stays clean and the
change adds no churn. Enable them at `warn` in `[workspace.lints.clippy]` in one change, then deny them once the
gate has run clean.

Before adding a name, check it against the toolchain's lint list (`cargo clippy -- -W help`). Some names in the
list are deprecated or renamed, and enabling those does nothing, so they are left out until they are checked.

alloc_instead_of_core as_pointer_underscore as_ptr_cast_mut as_underscore branches_sharing_code cargo_common_metadata
cfg_not_test checked_conversions clear_with_drain cloned_instead_of_copied coerce_container_to_any collapsible_else_if
collection_is_never_read comparison_chain copy_iterator create_dir debug_assert_with_mut_call
default_union_representation definition_in_module_root disallowed_script_idents doc_broken_link
doc_comment_double_space_linebreaks doc_include_without_cfg doc_link_code doc_link_with_quotes
doc_paragraphs_missing_punctuation elidable_lifetime_names empty_drop empty_enum_variants_with_brackets empty_enums
empty_structs_with_brackets enum_glob_use equatable_if_let expl_impl_clone_on_copy explicit_deref_methods
explicit_into_iter_loop fallible_impl_from filetype_is_file filter_map_next flat_map_option fn_params_excessive_bools
fn_to_numeric_cast_any format_collect future_not_send ignore_without_reason implicit_clone implicit_hasher
imprecise_flops inconsistent_struct_constructor index_refutable_slice inefficient_to_string infinite_loop
inline_always inline_asm_x86_att_syntax inline_asm_x86_intel_syntax into_iter_without_iter invalid_upcast_comparisons
ip_constant iter_filter_is_ok iter_filter_is_some iter_not_returning_iterator iter_on_empty_collections
iter_on_single_items iter_over_hash_type iter_with_drain iter_without_into_iter large_digit_groups large_futures
large_include_file large_stack_arrays large_stack_frames large_types_passed_by_value linkedlist
literal_string_with_formatting_args macro_use_imports manual_assert manual_bit_width manual_ilog2
manual_instant_elapsed manual_is_power_of_two manual_is_variant_and manual_let_else manual_string_new match_bool
match_wild_err_arm maybe_infinite_iter mismatching_type_param_order missing_fields_in_debug
mixed_read_write_in_expression mod_module_files mut_mut mutex_atomic mutex_integer needless_bitwise_bool
needless_collect needless_continue needless_raw_string_hashes needless_type_cast negative_feature_names
no_effect_underscore_binding no_mangle_with_rust_abi non_std_lazy_statics non_zero_suggestions nonminimal_bool
option_as_ref_cloned option_option overly_complex_bool_expr path_buf_push_overwrite pathbuf_init_then_push
pointer_format ptr_as_ptr ptr_cast_constness ptr_offset_by_literal pub_underscore_fields pub_without_shorthand
range_minus_one rc_buffer rc_mutex read_zero_byte_vec redundant_else redundant_feature_names redundant_test_prefix
ref_as_ptr ref_binding_to_reference ref_option ref_option_ref same_functions_in_if_condition same_length_and_capacity
same_name_method search_is_some self_only_used_in_recursion set_contains_or_insert should_panic_without_expect
single_char_pattern single_option_map str_split_at_newline string_add_assign string_lit_chars_any
suspicious_operation_groupings suspicious_xor_used_as_pow trailing_empty_array trait_duplication_in_bounds
transmute_ptr_to_ptr transmute_undefined_repr trivial_regex tuple_array_conversions type_repetition_in_bounds
unicode_not_nfc uninhabited_references unnecessary_debug_formatting unnecessary_join unnecessary_literal_bound
unnecessary_rest_pattern unnecessary_safety_comment unnecessary_safety_doc unnecessary_struct_initialization
unnecessary_trailing_comma unnecessary_wraps unnested_or_patterns unsafe_derive_deserialize unused_async
unused_async_trait_impl unused_rounding used_underscore_items useless_let_if_seq verbose_bit_mask volatile_composites
while_float wildcard_dependencies with_capacity_zero zero_sized_map_values

## Phase 6: correctness and panic-freedom

Each lint flags code that can hide an error or panic from a function that promises to return one. Warn first, then deny
once the sites are fixed.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `let_underscore_must_use` | warn | 62 | Each `let _ = fallible()` propagates with `?`, is handled, or is taken with `.ok()` and a comment that says why the error is ignored | not started |
| `panic_in_result_fn` | warn | 10 | Return `Err` where the function already returns `Result`; keep a panic only where the bound holds by construction, with an `expect` that says so | not started |
| `unreachable` | warn | 5 | Return an error or restructure the match so the branch is not needed; an `expect` only where the branch is provably unreachable | not started |
| `float_cmp` | warn | 10 | Review each exact `==` or `!=` on floats: keep it where the value is exact by construction (with an `expect` and a reason), compare with a tolerance, or compare bits | not started |
| `significant_drop_tightening` | warn | 10 | Drop the guard or lock before the work that does not need it | not started |
| `significant_drop_in_scrutinee` | warn | 2 | Bind the value and release the guard before the `match` or `if let` body runs | not started |
| `map_err_ignore` | warn | 17 | Keep the source error in the message or the error chain instead of discarding it with `\|_\|` | not started |
| `redundant_clone` | warn | 8 | Remove the clone where the original is not used afterwards | not started |
| `or_fun_call` | warn | 2 | `unwrap_or_else` or `ok_or_else` where the fallback does work | not started |
| `unchecked_time_subtraction` | warn | 1 | `checked_sub` or `saturating_duration_since` | not started |
| `case_sensitive_file_extension_comparisons` | warn | 1 | Compare the extension without case, as the media lookup does | not started |
| `use_debug` | warn | 1 | `{:?}` only for debugging output, not user-facing text | not started |
| `non_send_fields_in_send_ty` | warn | 2 | Confirm the `Send` impl, or drop it | not started |
| `error_impl_error` | warn | 2 | Rename the error type so it does not end in `Error` twice, or keep and allow with a reason | not started |
| `lossy_float_literal` | warn | 3 | Write the literal at full precision | not started |
| `unused_results` | warn | 291 | Bind or handle each non-`()` result; overlaps with `let_underscore_must_use` in this phase | not started |

## Phase 7: unsafe code (decision needed)

`unsafe_code` flags every `unsafe` block, function and impl. The workspace has 472 such sites. 446 are in `desktop`,
spread over 23 hand-written files (`gallery.rs` and `shell.rs` hold 170 between them), 20 are in `platform`, and the
rest are in `gallery`, `trace` and `video`. The policy needs deciding before any site changes.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `unsafe_code` | warn | 472 | Decide where `unsafe` may appear. Proposed: `#![forbid(unsafe_code)]` in every crate that has none (forbid is not an allow, so the policy holds), and a `#[expect(unsafe_code, reason = "...")]` on each module or item that needs it, with the reason naming the FFI call. Decide first whether `desktop` takes one crate-level expect with one reason, or an expect on each item; its sites are in hand-written files, so per-item expects are possible | decision needed |
| `borrow_as_ptr` | warn | 8 | `ptr::from_ref` or `ptr::from_mut` instead of `&x as *const _` | not started |
| `multiple_unsafe_ops_per_block` | warn | 58 | One operation per `unsafe` block, each with its own `// SAFETY:` line | not started |
| `cast_ptr_alignment` | warn | 1 | Read unaligned data with `read_unaligned`, or prove the alignment with a comment | not started |

## Phase 8: mechanical style fixes

Mechanical changes with no behaviour change. Warn, then deny once the sites are fixed.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `str_to_string` | warn | 95 | `"x".to_string()` becomes `"x".to_owned()` | not started |
| `clone_on_ref_ptr` | warn | 31 | `Arc::clone(&x)` or `Rc::clone(&x)` instead of `x.clone()` | not started |
| `redundant_closure_for_method_calls` | warn | 17 | Pass the method directly, such as `.map(str::len)` | not started |
| `derive_partial_eq_without_eq` | warn | 10 | Derive `Eq` alongside `PartialEq` where every field is `Eq` | not started |
| `semicolon_if_nothing_returned` | warn | 6 | Add the `;` to a block that returns `()` | not started |
| `match_same_arms` | warn | 5 | Merge arms that have the same body with `\|` | not started |
| `needless_pass_by_value` | warn | 5 | Take a reference where the value is not consumed | not started |
| `uninlined_format_args` | warn | 1 | `{name}` instead of `{}, name` | not started |
| `needless_raw_strings` | warn | 2 | Plain string where no escaping is needed | not started |
| `manual_midpoint` | warn | 4 | `f64::midpoint` or the integer equivalent | not started |
| `unreadable_literal` | warn | 8 | Digit separators in long numeric literals | not started |
| `explicit_iter_loop` | warn | 2 | `for x in &v` instead of `for x in v.iter()` | not started |
| `if_not_else` | warn | 2 | Invert the condition and swap the branches | not started |
| `items_after_statements` | warn | 9 | Move items to the top of the block | not started |
| `assert_is_empty` | warn | 18 | `assert!(v.is_empty())` gives a clearer failure message | not started |
| `unnecessary_box_returns` | warn | 9 | Return the value, not a `Box` | not started |
| `trivially_copy_pass_by_ref` | warn | 5 | Pass small `Copy` types by value | not started |
| `bool_to_int_with_if` | warn | 1 | `u8::from(cond)` | not started |
| `if_then_some_else_none` | warn | 1 | `then_some` or `then` | not started |
| `manual_assert_eq` | warn | 1 | `assert_eq!` | not started |
| `needless_pass_by_ref_mut` | warn | 1 | Take `&` where `&mut` is not needed | not started |
| `unnecessary_semicolon` | warn | 1 | Remove the `;` | not started |
| `deref_by_slicing` | warn | 25 | `&v[..]` becomes `v.as_slice()` or `v` | not started |
| `semicolon_outside_block` | warn | 13 | Move the `;` out of the block, or drop it where the value is `()` | not started |
| `semicolon_inside_block` | warn | 8 | Move the `;` inside the block so the block is a statement | not started |
| `assertions_on_result_states` | warn | 11 | Assert the value with `matches!` or `expect` in tests; the lint's own suggestion is `unwrap`, which Phase 3 bans outside tests | not started |
| `map_with_unused_argument_over_ranges` | warn | 11 | `(0..n).map(\|_\| x)` becomes `std::iter::repeat_n(x, n)` | not started |
| `shadow_same` | warn | 10 | Remove the rebinding of a name to the value it already has | not started |
| `return_and_then` | warn | 8 | `.and_then(\|x\| Some(y))` becomes `.map(\|x\| y)` | not started |
| `decimal_literal_representation` | warn | 6 | Write the large literal in hex, as the lint asks | not started |
| `self_named_module_files` | warn | 5 | Rename `foo/mod.rs` to `foo.rs`; the module directory stays | not started |
| `default_trait_access` | warn | 5 | `Default::default()` becomes `Type::default()` | not started |
| `ignored_unit_patterns` | warn | 5 | `Ok(_)` becomes `Ok(())` where the value is `()` | not started |
| `else_if_without_else` | warn | 4 | Add the final `else`, or restructure the chain | not started |
| `return_self_not_must_use` | warn | 3 | `#[must_use]` on the builder method that returns `Self` | not started |
| `inline_modules` | warn | 3 | Move the `mod name { }` body into its own file | not started |
| `separated_literal_suffix` | warn | 3 | `1u8` becomes `1_u8` | not started |
| `precedence_bits` | warn | 2 | Parenthesise mixed shift and bit operations | not started |
| `string_add` | warn | 2 | `a + &b` becomes `format!` or `push_str` | not started |
| `map_unwrap_or` | warn | 2 | `.map(f).unwrap_or(d)` becomes `.map_or(d, f)` | not started |
| `range_plus_one` | warn | 2 | `x..y + 1` becomes `x..=y` | not started |
| `renamed_function_params` | warn | 2 | Use the parameter names the trait declares | not started |
| `verbose_file_reads` | warn | 1 | Read the whole file with `std::fs::read` or `read_to_string` | not started |
| `format_push_string` | warn | 1 | `write!` into the `String` instead of `push_str(&format!(..))` | not started |
| `needless_for_each` | warn | 1 | A `for` loop instead of `for_each` | not started |
| `ref_patterns` | warn | 1 | Match on the reference, without `ref` | not started |
| `string_lit_as_bytes` | warn | 1 | `"..".as_bytes()` becomes `b".."` | not started |
| `unused_peekable` | warn | 1 | Remove `peekable()` where the iterator never peeks | not started |
| `wildcard_imports` | warn | 1 | Name the imported items | not started |
| `duration_suboptimal_units` | warn | 1 | `from_secs(60)` becomes `from_mins(1)` | not started |
| `unused_result_ok` | warn | 1 | Handle the error. Do not turn it into `.ok()` to discard it | not started |
| `match_wildcard_for_single_variants` | warn | 1 | Name the variants in the match | not started |
| `decimal_bitwise_operands` | warn | 1 | Write the literal in hex or binary | not started |
| `redundant_type_annotations` | warn | 1 | Remove the annotation the compiler infers | not started |
| `stable_sort_primitive` | warn | 1 | `sort_unstable` for primitives, where equal values cannot be told apart | not started |
| `unnecessary_self_imports` | warn | 1 | Remove the `self` from the `use` list | not started |
| `assigning_clones` | warn | 1 | `clone_from` instead of assigning a clone | not started |
| `used_underscore_binding` | warn | 1 | Remove the underscore, or stop using the binding | not started |

## Phase 9: arithmetic and floats (decision needed)

A change here can alter a result on overflow, rounding or a negative operand, so each site is checked, not swept. `integer_division` and `integer_division_remainder_used` ask the same question.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `integer_division` | warn | 87 | State the intent with `div_ceil`, `checked_div` or a comment; plain `/` stays where it is correct | decision needed |
| `arithmetic_side_effects` | warn | 642 | Checked, saturating or wrapping arithmetic at every site; the largest churn on this list, so scope it (for example to `pg-model`) before starting | decision needed |
| `integer_division_remainder_used` | warn | 175 | Adopt it, which means `checked_div`, `div_euclid` or `rem_euclid` at 175 sites, or leave it off; see Phase 9 | decision needed |
| `suboptimal_flops` | warn | 19 | `a * b + c` becomes `mul_add`. The result can differ in the last bit, so check each site | decision needed |
| `modulo_arithmetic` | warn | 6 | `%` with a possibly negative operand gives a negative result. Check each site; for timestamps and offsets, `rem_euclid` is usually what is meant | decision needed |
| `float_cmp_const` | warn | 1 | `==` against a constant. Use an exact comparison with a reason, or a tolerance | decision needed |

## Phase 10: public surface and binary parsing (decision needed)

These change the public surface or the parsing style. Each needs a decision in the Work column before it starts.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `exhaustive_structs` | warn | 104 | Decide whether public structs may grow fields; `#[non_exhaustive]` where they may | decision needed |
| `exhaustive_enums` | warn | 79 | Decide whether public enums may grow variants; `#[non_exhaustive]` where they may | decision needed |
| `redundant_pub_crate` | warn | 66 | Remove `pub(crate)` where the module is already private | decision needed |
| `pub_with_shorthand` | warn | 77 | `pub(in path)` style; decide the form once | decision needed |
| `pub_use` | warn | 31 | Decide whether re-exports are part of the API | decision needed |
| `unused_self` | warn | 36 | Remove `self` where a method does not use it, or make it an associated function | decision needed |
| `little_endian_bytes` | warn | 75 | Decide one endianness style for binary parsing (explicit `from_le_bytes` everywhere) | decision needed |
| `big_endian_bytes` | warn | 40 | Same decision as above | decision needed |
| `host_endian_bytes` | warn | 13 | Host-endian reads are only correct for data the app itself wrote; check each one | decision needed |
| `module_name_repetitions` | warn | 20 | Item names that repeat the module name (`model::ModelX`). Renaming public items changes the API | decision needed |
| `struct_field_names` | warn | 1 | A field name that repeats the struct name. Renaming a public field changes the API | decision needed |
| `partial_pub_fields` | warn | 3 | A struct with some public fields and some private. Make them all public or all private | decision needed |
| `field_scoped_visibility_modifiers` | warn | 7 | `pub(in path)` on fields. Decide with `pub_with_shorthand` above | decision needed |

## Phase 11: code shape (decision needed)

How code is written: match arms, shadowing, function length and bool fields. Each lint needs a decision before its sites change.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `wildcard_enum_match_arm` | warn | 27 | Name every variant in a match; new variants then break the build, which is the point | decision needed |
| `shadow_reuse` | warn | 265 | Adopt it, which means a new name for each intermediate value at 265 sites, or leave it off | decision needed |
| `too_many_lines` | warn | 7 | Functions over the line limit. Split them, or allow with a reason | decision needed |
| `cognitive_complexity` | warn | 5 | Functions over the complexity threshold. Split them, or allow with a reason | decision needed |
| `struct_excessive_bools` | warn | 4 | Several `bool` fields. Replace with an enum or a bitflags type where the states are exclusive | decision needed |
| `missing_asserts_for_indexing` | warn | 4 | An `assert!` on the length before the index. The assert adds a panic path, so check each site | decision needed |

## Phase 12: derives, dependencies, dead code and visibility

Removing an item or a dependency can break a build that relies on it, so each row names what to check.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `missing_debug_implementations` | warn | 91 | `#[derive(Debug)]` where every field implements `Debug`; a manual impl for a type that holds a handle or a secret | not started |
| `missing_copy_implementations` | warn | 19 | `#[derive(Clone, Copy)]` on small types that have no heap data | not started |
| `variant_size_differences` | warn | 4 | Box the large variant, or split the enum, where one variant dwarfs the rest | not started |
| `unused_crate_dependencies` | warn | 27 | Remove the dependency from the crate's `Cargo.toml`; check the feature flags it enabled | not started |
| `elided_lifetimes_in_paths` | warn | 42 | Write `Foo<'_>` in signatures where a lifetime is elided | not started |
| `let_underscore_drop` | warn | 49 | Name the value and drop it explicitly with `drop(x)`, or let it go out of scope | not started |
| `clippy::multiple_crate_versions` | warn | 3 | Align the two versions in `Cargo.toml`, or record why both are needed | not started |
| `dead_code_pub_in_binary` | warn | 284 | Remove the item, or make it private if only the binary uses it. The largest counts are in `model` (58) and `client` (49) | not started |
| `redundant_imports` | warn | 7 | Remove the import that the prelude or an earlier `use` already provides | not started |
| `unreachable_pub` | warn | 157 | `pub(crate)` for items that are not used outside their crate | not started |
| `naive_bytecount` | warn | 1 | Counting bytes with `filter` and `count`. The fix adds the `bytecount` crate | decision needed |

Not measured: eight names in the rustc list were not recognised by the compiler cargo used, so they have no hit count:
`deprecated_llvm_intrinsic`, `implicit_provenance_casts`, `multiple_supertrait_upcastable`, `must_not_suspend`,
`non_exhaustive_omitted_patterns`, `resolving_to_items_shadowing_supertrait_items`, `shadowing_supertrait_items` and
`unqualified_local_imports`. Check them against the toolchain before adding any of them to a phase.

## Phase 13: disallowed methods, types and macros (decision needed)

`disallowed_methods`, `disallowed_types` and `disallowed_macros` ban specific calls in specific crates, configured in
`clippy.toml`. They enforce a boundary rather than a style. Each ban needs a decision in the Work column before it is
added.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `disallowed_methods` | warn | not counted | Ban clock, file and process calls in `model`, which the library docs describe as pure and WASM-safe (for example `SystemTime::now`, `std::fs`) | decision needed |
| `disallowed_types` | warn | not counted | Ban platform types in crates that must not hold them (for example `std::process::Command` outside `platform` and the tools) | decision needed |
| `disallowed_macros` | warn | not counted | Ban `println!` and `eprintln!` in library crates; overlaps with Phase 2, so only if Phase 2 leaves a gap | decision needed |

## Phase 14: not recommended

Recorded so the decision is visible. None of these is planned. Each row says why the lint is excluded, and a lint moves
here only with a decision recorded in the Work column.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `float_arithmetic` | none | 195 | Flags every float operation, which is the point of a graphics pipeline; excluded | not planned |
| `shadow_unrelated` | none | 126 | Flags rebinding a name to an unrelated value; common and readable in this code; excluded | not planned |
| `multiple_inherent_impl` | none | 32 | Flags splitting one type's methods across impl blocks, which the code does on purpose by module; excluded | not planned |
| `missing_assert_message` | none | 11 | Asks for a message on every test assertion; low value for the cost; excluded | not planned |
| `use_self` | none | 424 | Rewrites type names to `Self`; churn without a correctness gain; excluded | not planned |
| `must_use_candidate` | none | 338 | Asks for `#[must_use]` on many functions; excluded until an API needs it | not planned |
| `missing_errors_doc` | none | 190 | Requires an `# Errors` section on every fallible public function; documentation debt; excluded | not planned |
| `too_long_first_doc_paragraph` | none | 128 | Doc-comment layout only; excluded | not planned |
| `missing_const_for_fn` | none | 110 | Asks for `const` on functions that could be const; churn; excluded | not planned |
| `doc_markdown` | none | 87 | Wants identifiers in backticks in doc comments; excluded | not planned |
| `missing_panics_doc` | none | 74 | Requires a `# Panics` section; documentation debt; excluded | not planned |
| `implicit_return` | none | 3166 | Requires an explicit `return` on every tail expression; excluded | not planned |
| `question_mark_used` | none | 1426 | Flags `?`, which is the idiom; excluded | not planned |
| `min_ident_chars` | none | 2044 | Flags short names, which the code uses for indexes and coordinates; excluded | not planned |
| `arbitrary_source_item_ordering` | none | 1119 | Enforces an item order no one follows; excluded | not planned |
| `absolute_paths` | none | 763 | Requires `use` for every path; excluded | not planned |
| `default_numeric_fallback` | none | 487 | Requires annotated literal types; excluded | not planned |
| `single_call_fn` | none | 368 | Flags helper functions with one caller; those helpers name steps; excluded | not planned |
| `missing_docs` | none | 764 | Documentation debt on the public surface; excluded | not planned |
| `missing_docs_in_private_items` | none | 812 | Documentation debt on private items; excluded | not planned |
| `missing_inline_in_public_items` | none | 797 | Asks for `#[inline]` on public functions; excluded | not planned |
| `std_instead_of_core` | none | 167 | Prefers `core` paths; excluded | not planned |
| `std_instead_of_alloc` | none | 45 | Prefers `alloc` paths; excluded | not planned |
| `pattern_type_mismatch` | none | 146 | Requires explicit reference patterns; excluded | not planned |
| `non_ascii_literal` | none | 115 | Flags non-ASCII text, which the UI strings use; excluded | not planned |
| `unseparated_literal_suffix` | none | 139 | Numeric suffix style; excluded | not planned |
| `unused_trait_names` | none | 80 | Requires `as _` for trait imports; excluded | not planned |
| `tests_outside_test_module` | none | 55 | Requires tests in a `mod tests`; the integration tests are separate by design; excluded | not planned |
| `let_underscore_untyped` | none | 71 | Requires a type on `let _`; excluded | not planned |
| `rest_pattern_accessible_field` | none | 69 | Requires naming every field in a pattern; excluded | not planned |
| `inline_trait_bounds` | none | 51 | Trait-bound layout; excluded | not planned |
| `impl_trait_in_params` | none | 31 | Argument-position `impl Trait` style; excluded | not planned |
| `single_char_lifetime_names` | none | 32 | Lifetime naming; excluded | not planned |
| `many_single_char_names` | none | 28 | Flags short names in one scope; excluded with `min_ident_chars` | not planned |
| `similar_names` | none | 16 | Flags similar names; excluded | not planned |
| `option_if_let_else` | none | 18 | Prefers `map_or` over `if let`; excluded | not planned |
| `single_match_else` | none | 10 | Prefers `if let` over a two-arm `match`; excluded | not planned |
| `missing_trait_methods` | none | 12 | Requires every provided trait method to be listed; excluded | not planned |

The whole restriction group is not enabled. Clippy's `blanket_clippy_restriction_lints` lint warns against enabling it,
and each lint is enabled on its own merits.

The first four hit counts are from the current tree. The last seven are from the baseline counts above.

## Test code

Test code is excluded from the counts above. Allow the panic-family lints there through `clippy.toml`:

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-indexing-slicing-in-tests = true
allow-panic-in-tests = true
allow-print-in-tests = true
allow-dbg-in-tests = true
```

Status: `allow-panic-in-tests` and `allow-print-in-tests` are set in the root `clippy.toml` and in the two crate files that
replace it. They cover `#[test]` functions; helpers still need an `expect` (see Phase 1). `allow-unwrap-in-tests`,
`allow-expect-in-tests` and `allow-indexing-slicing-in-tests` are set too (Phase 3). `allow-dbg-in-tests` is not set;
confirm it with `cargo clippy` before adding it.

## Proposed first configuration

For `Cargo.toml` `[workspace.lints.clippy]`, after the Phase 1 work is done:

```toml
undocumented_unsafe_blocks = "warn"
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
mem_forget = "deny"
exit = "deny"
panic = "deny"
allow_attributes = "warn"
string_slice = "warn"
```

## Open decisions

1. Phase 1 as a single change, or one lint per change. Decided: a single change, uncommitted, pending review.
2. The print lints: which crates may print to stdout or stderr. Decided: see Phase 2.
3. Whether `as_conversions` applies to `pg-model` only.
4. Whether the cast lints go in before or after the panic-freedom lints.

## Method

- - Linux-only code (`#[cfg(target_os = "linux")]`, and the bench `ir/linux.rs`, which `ir.rs` includes on Linux) is not
  compiled by the Windows gate. Check it in WSL with the Linux CI job's commands (see Phase 3).
- Counts come from `cargo clippy --workspace --lib --bins --message-format=json` with the candidate lints passed as
  `-W`. Diagnostics are deduplicated by lint, file, line and message.
- A lint is done when it is enabled at its level in `Cargo.toml` and `just lint` passes.
