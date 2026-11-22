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
- `[workspace.lints.rust]` sets `unreachable_pub` at warn (Phase 12, `e1477ee`), `missing_docs` at warn (Phase 9 of the consistency program: every public item and field has a doc comment) and `unsafe_code` at deny (Phase 8 of the consistency program). The crates without unsafe code say `#![forbid(unsafe_code)]` at their root; an unsafe item carries `#[expect(unsafe_code, reason)]`. `UnsafePolicy` in `tools/test_ci_files.py` checks both.
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

Phases 5 to 14 group the remaining lints by the work each one needs: enable now, correct, decide, or leave off.

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
| the 171 names below | deny | 0 | Add each name at `deny`; no code change expected | not started |

These 171 lints are not in the default set, have zero hits in the current tree on Windows and WSL, and are not named
elsewhere in this ledger. Enabling them is configuration only: each has zero sites, so the gate stays clean and the
change adds no churn. Enable them at `deny` in `[workspace.lints.clippy]` in one change. A `warn` stage does not fit the
gate: `just lint` runs clippy with `-D warnings` (see Current configuration), so a warn-level lint fails it. With zero
hits, `deny` passes the gate directly.

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

## Phase 7: unsafe code (decided)

`unsafe_code` flags every `unsafe` block, function and impl. The baseline run counted 472 lint hits (compiler diagnostics,
not source lines; the grep counts are in `VALUE_PROVENANCE_IDEAS.md` and `NIGHTLY_AND_BUILD_FLAGS_PLAN.md`). 446 hits are in `desktop`,
spread over 23 hand-written files (`gallery.rs` and `shell.rs` hold 170 between them), 20 are in `platform`, and the
rest are in `gallery`, `trace` and `video`. The policy is recorded in the table below.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `unsafe_code` | deny | 472 | Done. The workspace denies it; the 30 crate roots without `unsafe` say `#![forbid(unsafe_code)]`. The five crates with unsafe (`dedup`, `desktop`, `platform`, `trace`, `video`) say `#[expect(unsafe_code, reason)]` on each item, or on the module when all of its unsafe is one FFI layer (`platform`, `video`'s `ffi`). A cxx-qt bridge module carries its expect on its declaration in `desktop/src/lib.rs`. No crate-level expect | done (Phase 8) |
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
| `assertions_on_result_states` | warn | 11 | Assert the value with `matches!` or `expect` in tests; the lint's own suggestion is `unwrap`, which `unwrap_used` bans outside tests | not started |
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

## Phase 9: arithmetic and floats (decided)

A change here can alter a result on overflow, rounding or a negative operand, so each site is checked, not swept. `integer_division` and `integer_division_remainder_used` ask the same question.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `integer_division` | warn | 87 | Adopt. Per site: `div_ceil` where rounding up is meant, `checked_div` where the divisor can be zero, `#[expect]` with a reason where truncation is the intent | decided: adopt |
| `arithmetic_side_effects` | warn | 642 | Adopt workspace-wide at `deny`, test code included. Checked arithmetic that propagates the error, or `saturating_*` for counters; wrapping only where the bits are the value | decided: adopt |
| `integer_division_remainder_used` | warn | 175 | Adopt. Fires on unsigned `/` and `%` too (probe). `checked_div` and `checked_rem` where the divisor can be zero (a panic path); `div_euclid` and `rem_euclid` where the operand can be negative | decided: adopt |
| `suboptimal_flops` | warn | 19 | Adopt. `mul_add` at each site. The result can differ in the last bit; the golden-output and pixel tests decide whether a result moved, and any change is listed in the commit | decided: adopt |
| `modulo_arithmetic` | warn | 6 | Adopt. `rem_euclid` for timestamps and offsets; `#[expect]` with a reason where the operand is known non-negative | decided: adopt |
| `float_cmp_const` | warn | 1 | Adopt. Exact comparison only against a sentinel that is exact by construction, with `#[expect]` and a reason; a tolerance everywhere else | decided: adopt |

## Phase 10: public surface and binary parsing (decided)

These change the public surface or the parsing style. Each decision is recorded in the Status column.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `exhaustive_structs` | warn | 104 | Not adopted. Internal crates have no outside consumers, and `#[non_exhaustive]` blocks struct literals across crates | decided: not adopted |
| `exhaustive_enums` | warn | 79 | Not adopted. `#[non_exhaustive]` forces `_` arms in other crates, which conflicts with `wildcard_enum_match_arm` (Phase 11) | decided: not adopted |
| `redundant_pub_crate` | warn | 66 | Not adopted. It conflicts with `unreachable_pub`, which is enabled workspace-wide (see Current configuration): `pub(crate)` for items not used outside their crate | decided: not adopted |
| `pub_with_shorthand` | warn | 77 | Not adopted. The probe shows it fires on `pub(crate)` and `pub(super)` and prefers `pub(in path)`, which is less common. A style call | decided: not adopted |
| `pub_use` | warn | 31 | Not adopted. Re-exports are a deliberate facade | decided: not adopted |
| `unused_self` | warn | 36 | Adopt. Remove the `self`, or make the method an associated function, and update the call sites | decided: adopt |
| `little_endian_bytes` | warn | 75 | Not adopted. The probe shows it fires on every `from_le_bytes` and `to_le_bytes`, the explicit form the policy requires; satisfying it would take an `#[expect]` at each site | decided: not adopted |
| `big_endian_bytes` | warn | 40 | Not adopted, as `little_endian_bytes`. The probe shows it fires on `from_be_bytes` | decided: not adopted |
| `host_endian_bytes` | warn | 13 | Adopt. The probe shows it fires on `from_ne_bytes`. Change each to explicit LE or BE; `#[expect]` with a reason only where the data never leaves the process | decided: adopt |
| `module_name_repetitions` | warn | 20 | Adopt. Rename the items; the crates are internal, so the cost is churn | decided: adopt |
| `struct_field_names` | warn | 1 | Adopt. Rename the field | decided: adopt |
| `partial_pub_fields` | warn | 3 | Adopt per site. Default to all private; public only where a struct is built by literal outside its module | decided: adopt |
| `field_scoped_visibility_modifiers` | warn | 7 | Not adopted, as `pub_with_shorthand` | decided: not adopted |

## Phase 11: code shape (decided)

How code is written: match arms, shadowing, function length and bool fields. Each decision is recorded in the Status column.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `wildcard_enum_match_arm` | warn | 27 | Adopt. Name every variant in a match; a new variant then breaks the build | decided: adopt |
| `shadow_reuse` | warn | 265 | Not adopted. Rebinding to a derived value is idiomatic Rust, and the lint would rename 265 intermediates for no gain | decided: not adopted |
| `too_many_lines` | warn | 7 | Adopt. Split the functions; `#[expect]` with a reason only where a dispatch match is clearer whole | decided: adopt |
| `cognitive_complexity` | warn | 5 | Adopt. Split the functions; the same rule as `too_many_lines` | decided: adopt |
| `struct_excessive_bools` | warn | 4 | Adopt per site. An enum where the states are exclusive; `#[expect]` with a reason where the flags are independent | decided: adopt |
| `missing_asserts_for_indexing` | warn | 4 | Adopt. Fix by restructuring (`get`, `let else`, a slice pattern), not by adding an `assert!`, which is a new panic path | decided: adopt |

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
| `naive_bytecount` | warn | 1 | Adopt. Use `memchr::memchr_iter(..).count()` if the crate already depends on `memchr`; otherwise keep the loop with `#[expect]` and a reason. No new dependency for one site | decided: adopt |

Not measured: eight names in the rustc list were not recognised by the compiler cargo used, so they have no hit count:
`deprecated_llvm_intrinsic`, `implicit_provenance_casts`, `multiple_supertrait_upcastable`, `must_not_suspend`,
`non_exhaustive_omitted_patterns`, `resolving_to_items_shadowing_supertrait_items`, `shadowing_supertrait_items` and
`unqualified_local_imports`. Check them against the toolchain before adding any of them to a phase.

## Phase 13: disallowed methods, types and macros (decided)

`disallowed_methods`, `disallowed_types` and `disallowed_macros` ban specific calls in specific crates, configured in
`clippy.toml`. They enforce a boundary rather than a style. Each decision is recorded in the Status column.

| Lint | Level | Hits | Work | Status |
|---|---|---|---|---|
| `disallowed_methods` | warn | not counted | Adopt at `deny` in `crates/model/clippy.toml`, banning `SystemTime::now` and `std::fs`, since the library docs describe `model` as pure and WASM-safe. That file replaces the root one, so the root's settings are repeated there | decided: adopt |
| `disallowed_types` | warn | not counted | Adopt at `deny` workspace-wide, banning `std::process::Command`. Each place that spawns a process (`pg-exiftool`, `video`, the platform process code, the tools) takes `#[expect(clippy::disallowed_types, reason = "...")]`. Crates with their own `clippy.toml` repeat the ban | decided: adopt |
| `disallowed_macros` | warn | not counted | Not adopted. `print_stdout` and `print_stderr` already deny the same macros | decided: not adopted |

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
| `missing_docs` | warn (workspace) | 764 | Every public item and field documented; `just lint` makes it fatal (AD-004) | done (Phase 9) |
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
```

## Method

- - Linux-only code (`#[cfg(target_os = "linux")]`, and the bench `ir/linux.rs`, which `ir.rs` includes on Linux) is not
  compiled by the Windows gate. Check it in the container (`just check-linux`), with the Linux CI job's commands.
- Counts come from `cargo clippy --workspace --lib --bins --message-format=json` with the candidate lints passed as
  `-W`. Diagnostics are deduplicated by lint, file, line and message.
- A lint is done when it is enabled at its level in `Cargo.toml` and `just lint` passes.
