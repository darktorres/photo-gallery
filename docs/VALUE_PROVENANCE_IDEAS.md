# Tracking values end to end: ideas for later

Status: ideas only, not planned work. Nothing here is implemented, and no tool named below is installed in this repo's toolchain.

## The question

Can we track a value from where it enters the program (a file's bytes, a QML property) to where it is used (a SQL bind, a slice, a QML property), the way Psalm's taint analysis tracks untrusted input to its sinks in PHP?

## What Rust has

Rust has no built-in taint analysis. These are the closest options, in the order to try them.

### 1. Types, enforced by rustc (the most proper option here)

Wrap values that come from outside the program in a type with no `Deref`, `Into` or `From`. Its only way out is a validator that returns the domain type.

- `Untrusted<u64>` comes out of the TIFF, EXIF and XMP readers.
- The only exit is `checked`, a `try_from` that returns a domain type: `UserRating`, `usize`, an orientation enum.
- Sinks accept domain types only: SQL binds, slice indexes, QML properties.

Rustc then checks the whole chain at compile time. There is no analysis run and no runtime cost.

Limit: this tracks type-level provenance, not values. It shows that a file value went through a validator. It does not show that the validator is correct.

### 2. Lints that enforce the boundary

The repo already denies `as_conversions`, `cast_possible_truncation` and `cast_possible_wrap` (`Cargo.toml`, lines 86 to 94), so no silent `as` cast sits between the layers.

`clippy.toml` already bans `HashMap` with `disallowed-types`. The same mechanism can ban the escape hatches, such as an `into_inner()` on the wrapper outside its own module.

### 3. Whole-program dataflow

Tools such as MIRAI and Flowistry track flows across functions. They are research-grade. Their maintenance status has not been checked, so do not build on them without checking first.

Verification tools (Kani, Prusti, Creusot) can prove a property of a single conversion, such as "this boundary never truncates". None is installed.

## Limits of all of these

- `unsafe` code and FFI are not seen. This includes the cxx-qt bridge.
- QML is untyped JavaScript. A value's journey stops at the Rust receiver.
- A `serde` impl can build a domain type straight from input. That is only safe when the impl goes through the validator (`Rating`'s `try_from = "u8"` does).

## Where to start

Option 1 in the metadata crate (`crates/metadata`). File-derived values from TIFF, EXIF and XMP enter as `Untrusted`, and leave only through a validator. Option 2 then keeps the boundary from regressing.

Open questions:

- Which conversion ladder should the metadata crate use: file bytes to `u64` to domain type?
- Should the same wrapper cover values from QML, or only from files?
- Is a prototype wanted first, or a plan in `docs/`?
