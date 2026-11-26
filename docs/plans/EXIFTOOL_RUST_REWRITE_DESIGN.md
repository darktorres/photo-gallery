# A standalone Rust reimplementation of ExifTool

> **Status (2026-11-26).** Design only; nothing is built. It replaces the port plans (`EXIFTOOL_PORT_PLAN.md`,
> `EXIFTOOL_PORT_DESIGN.md`, removed), which ported ExifTool's Perl one-for-one into photo-gallery. The engine is a
> standalone crate in a repository of its own; this file moves there when the repository is made. Figures are marked
> **measured** (callgrind, this repo's container, against the exiftool-rs fork at `D:\GitHub\exiftool-rs`, commit
> 9d78ca4) or **estimate**. The reference is Perl ExifTool 13.64 (`D:\GitHub\exiftool`, tag `13.64`).

## Goal

A Rust library, and a command-line tool on top of it, that reads and writes the metadata of every format ExifTool
handles, with ExifTool's knowledge of tags and vendor quirks, at the cost of a purpose-built reader.

It is a reimplementation, not a port. It reproduces ExifTool's *values*, checked by differential tests against Perl
ExifTool. It does not reproduce ExifTool's execution model, its internal data structures, or its output text, except
through an optional compatibility layer.

It is general-purpose. It knows nothing of photo-gallery; photo-gallery is one of its consumers.

## Why a reimplementation

exiftool-rs runs ExifTool's own modules (dumped Perl data, Perl table code, ported subs) on a Perl interpreter written
in Rust. Its output is ExifTool's. Its cost is ExifTool's design, **measured** on warm reads (interpreter and modules
already loaded), in instructions per read:

| File | A purpose-built reader (photo-gallery's pg-metadata, a few fields) | exiftool-rs, nine tags | exiftool-rs, all tags |
|---|---:|---:|---:|
| camera.jpg | 55.7K | 14.0M | 17.8M |
| sample.jpg | 0.9K | 2.3M | 2.7M |
| ExifTool.jpg | 253K | 31.2M | 35.7M |

Four rounds of optimization in the fork took its write case from 17.45G to 6.79G instructions and its warm reads down
19–26%; the gap that remains is structural:

1. **More work.** ExifTool decodes every tag of every directory, then filters to what was asked (`SetFoundTags`); it
   parses maker notes whole (`ProcessBinaryData`: 28% of a camera.jpg read); it evaluates every composite definition
   (`Composite => 0` alone saves 8–30% per read, **measured**).
2. **Each step costs more.** Tag definitions are Perl hashes looked up by byte strings (about 80–115 instructions per
   lookup, about 30,000 lookups per camera.jpg read); every value is a reference-counted Perl scalar (about 37,000
   allocations per read); recording one tag (`FoundTag`) costs about 12,500 instructions; subs dispatch by name through
   the interpreter (about 420 instructions of bookkeeping per call); conditions and conversions are Perl strings run
   by the interpreter, each eval declaring 11–20 variables.

Both follow from running ExifTool's design, so the engine does not.

## Principles

- **Ask, then read.** The caller names tags, groups, or everything. A planner turns that into the directories to
  parse: maker notes, thumbnails, ICC profiles and video sample tables are parsed only when something asked for needs
  them.
- **Borrow, decode late.** Parsers work over a byte source and return values that borrow from it. The typed meaning
  is decoded when asked for; the printed text is a separate, last step.
- **Numbers, not strings.** Tags, groups and families are numeric identifiers with static name tables. A lookup is a
  match arm or an array index.
- **No interpreter.** Conditions and conversions are Rust: generated where their shape allows, written by hand where
  not, never evaluated from text.
- **Data in, code out.** ExifTool's knowledge enters as reviewed data files, generated into Rust by a build step.
  Generated code is never edited by hand.
- **Bounded and safe.** No panic, no unbounded loop or allocation on any input; limits on depth, sizes and counts;
  every parser fuzzed. Memory per file is bounded by that file's metadata.
- **No unsafe** outside a few audited places with a stated reason (none planned).

## Repository and crates

A workspace of its own:

| Crate | Role |
|---|---|
| `schema-data/` (not a crate) | The tag data, one file per ExifTool module, checked in and reviewed |
| `*-schema-gen` | Reads `schema-data/` and writes the generated Rust (build step) |
| `*-dump` (Perl script) | Dumps ExifTool's tables from Perl ExifTool into `schema-data/` |
| `*` (the library) | Containers, decoders, generated tables, conversions, composites, reader, writer |
| `*-cli` | A command-line tool; ExifTool-style output through the compatibility layer |
| `*-difftest` | The differential harness against Perl ExifTool |
| `fuzz/` | cargo-fuzz targets, one per container and decoder |

(`*` is the crate's name, still to choose; per exiftool-rs's rule, nothing is named `exiftool`.)

The library depends on no photo-gallery crate. Its public types are its own.

## Architecture

### Byte sources

A `Source` trait: random-access reads by offset and length, with a known length. Implementations for a slice, for a
memory map and for a file read by range. Parsers ask for what they need, so a video's box headers are read without
its media, and a JPEG's thumbnail is never read unless asked for.

### Containers

One module per container: JPEG segments, TIFF/IFD, PNG chunks, ISOBMFF (MP4, QuickTime, HEIF, AVIF, CR3, JXL), RIFF
(WebP, AVI, WAV), EBML (Matroska), GIF, PDF, and the long tail. Each is an iterator over its blocks yielding kinds,
offsets and lengths, never decoded data. Designed fresh for these requirements; the handling of odd files in existing
readers (photo-gallery's pg-container and pg-metadata) carries over as test cases.

### Directory decoders

- One generic IFD decoder: EXIF, GPS, Interop, IFD1, DNG, and the maker notes laid out as IFDs.
- One generic binary-table decoder: fixed offsets and formats per entry, as most maker notes and many vendor blocks
  are.
- A streaming XMP/RDF reader, IPTC (IIM), ICC, Photoshop IRB.
- One module per maker-note family for what is not generic: offset bases, encryption (Nikon, Sony), vendor framing.

### The schema

Generated from `schema-data/`:

- Per tag: identifier, name, format, count, groups (all families), writability, its table.
- Its conversion: a value table (a generated `match`, or a perfect-hash map for large ones), a typed formula
  (`Scale(1, 10)`, `Log2Exposure`, `Ascii`), or a named Rust function.
- Its condition, as a typed predicate: `MakeMatches(..)`, `ModelIn(..)`, `CountIs(n)`, `FormatIs(..)`,
  `BytesStartWith(..)`, or a named Rust function.
- Table selection (one maker-note layout per model family) is a `match` over predicates, decided once per directory,
  not per tag.

### Semantic types

The decoded meaning of a value is a type, not a string: `DateTime` (with an optional offset and subseconds, as the
standards allow), `Orientation`, `Rational`, `GpsCoordinate`, `Rating`, `LensSpec`, and so on. Printed text comes
from these.

### Composites

A dependency graph built at compile time: each composite lists the tags it requires and desires. A composite is
computed only when asked for, from tags that were read; nothing walks every composite definition per file.

### Results

Per file, an arena of entries: tag identifier, group, raw value (borrowed), decoded value on demand. Lookup by tag,
iteration in file order, filters by group. Results are `Send`; a reader is shareable across threads; each file's
reading is independent, so a caller parallelizes across files.

```rust
let reader = Reader::new().want([Tag::Orientation, Tag::DateTimeOriginal, Tag::XmpRating]);
let meta = reader.read(&source)?;
let orientation: Option<Orientation> = meta.get(Tag::Orientation).and_then(|e| e.decode());
for e in meta.iter() {
    println!("{} {} = {}", e.group(), e.name(), e.print());
}
```

### Errors and warnings

Errors only for what stops reading (an unreadable source, a container too damaged to walk). Everything else is a
warning: a typed enum with the container, the directory and the offset it refers to. Damaged directories are skipped
with a warning; a file always yields what could be read.

### Text

Character sets as the standards and ExifTool's options define them (EXIF's ASCII and its common UTF-8 misuse, IPTC's
coded character set, XMP's UTF-8/16/32, QuickTime's language-tagged strings), decoded to Rust strings with the
replacement policy stated per field; the raw bytes stay available.

### Writing

Separate from reading. The editing model is patches per directory; the container is rewritten with offsets
recomputed and unknown data preserved, and written atomically (a temporary file and a rename). Maker-note offset
fixups are vendor-specific and explicit, never guessed. Formats are added to writing only with their round-trip and
differential tests.

### Compatibility layer

Optional: ExifTool's tag names, group names and printed values, and its `-j`/`-s`/`-G` style output in the CLI, for
anyone who wants ExifTool-like output. A layer over the engine's results; it does not constrain the engine.

## ExifTool's knowledge

The knowledge (tens of thousands of tags, decades of vendor quirks) is ExifTool's real value; it is converted, not
retyped.

1. **Dump.** A Perl script loads each ExifTool module with Perl ExifTool 13.64 and writes its tables to
   `schema-data/` in a neutral, line-oriented format: one tag per record, with its fields, its value tables, and its
   conditions and conversions as Perl source text, verbatim.
2. **Classify.** The generator maps each condition and conversion to a typed form by its shape: Make/Model patterns,
   count and format checks, byte sniffing, lookup tables, linear and logarithmic formulas. What does not match a known
   shape goes to a list of hand ports. The list's size is unknown until the first dump (**estimate**: a small
   fraction).
3. **Hand ports.** Real code (`ConvertExifDate`, lens identification, decryption, the composite calculations) becomes
   Rust functions in modules of their own, one per ExifTool sub, each with tests against ExifTool's results. The
   generator refers to them by name.
4. **New releases.** Run the dump with the new release; review the diff of `schema-data/`; port what the generator
   cannot classify. The schema records the ExifTool version it came from.

**Licensing: a decision before phase 1.** Data dumped from ExifTool's tables is a derived work of ExifTool, which is
under the Perl terms (GPL or Artistic). The engine's licence follows from that choice and must be settled before any
data is committed.

## Correctness

- **Differential tests** against Perl ExifTool 13.64: the engine's values against ExifTool's, compared normalized (by
  tag and group, typed where the engine's type is known), on ExifTool's `t/images`, on files the tests generate, and on
  a corpus of real files. Every difference is fixed or accepted with a recorded reason. Zero unexplained differences is
  each phase's bar. exiftool-rs may stand in for Perl to run faster, once it has been shown to agree with Perl on the
  corpus.
- **Fuzzing** of every container and decoder, with the limits above, in CI.
- **Round trips** for writing: write, read back, compare; and ExifTool reads the written file the same.

## Performance

- **Expected:** full extraction of a camera JPEG with maker notes in the low hundreds of thousands of instructions
  (**estimate**, from pg-metadata's measured per-field costs), against exiftool-rs's 17.8M.
- **Measured** from the first phase: gungraun (callgrind) benches per container and per directory decoder, with
  budgets per phase; a regression beyond a budget fails CI.

## API stability

Semantic versioning. Tag identifiers are stable across releases (new tags get new identifiers; a tag ExifTool renames
keeps its identifier and gains an alias). The generated schema's types are public; the generator's are not.

## Phases

Each phase ends with its differential tests at zero unexplained differences, its fuzz targets clean in CI, and its
benches within budget.

1. **Core.** The repository and crates; the dump and the generator; byte sources; containers JPEG, TIFF, PNG, GIF and
   WebP; EXIF, XMP and IPTC; reading only; the differential harness; the CLI's basic output.
2. **Maker notes**, by how common they are: Canon, Nikon, Sony, Fujifilm, OM System/Olympus, Panasonic, Apple.
3. **Video and modern containers:** ISOBMFF (MP4, QuickTime, HEIF, AVIF, CR3, JXL), Matroska.
4. **Composites**, as the dependency graph.
5. **Writing**, for phase 1's formats, HEIF and DNG.
6. **The long tail of formats**, in the order the corpus shows them.

## Consumers

- **photo-gallery.** Its import calls the engine through an adapter in pg-metadata that maps the engine's semantic
  types to pg-model's. Once the engine covers the import's fields with zero differences, pg-metadata's own readers are
  retired and its tests move to the engine's suite as regression cases.
- **exiftool-rs** is not a consumer; it is a possible fast stand-in for Perl in the differential tests (above).

## Open questions

- The licence (above).
- The crate's name.
- The schema-data file format (line-oriented text so diffs review well, versus a structured format such as RON);
  decided with the first dump.
