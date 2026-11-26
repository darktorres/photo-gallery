# A standalone Rust reimplementation of ExifTool

> **Status (2026-11-26).** Design only; nothing is built. Revised after an adversarial review the same day (its
> corrections are folded in; the counts it rests on are in "ExifTool by the numbers"). It replaces the port plans
> (`EXIFTOOL_PORT_PLAN.md`, `EXIFTOOL_PORT_DESIGN.md`, removed), which ported ExifTool's Perl one-for-one into
> photo-gallery. The engine is a standalone crate in a repository of its own; this file moves there when the
> repository is made. Figures are marked **measured** (callgrind in photo-gallery's container, against the exiftool-rs
> fork at commit 9d78ca4), **counted** (in ExifTool 13.64's `lib/`), or **estimate**. The reference is Perl ExifTool
> 13.64.

## Goal

A Rust library, and a command-line tool on top of it, that reads and writes the metadata of every format ExifTool
handles, with ExifTool's knowledge of tags and vendor quirks, at the cost of a purpose-built reader.

It is a reimplementation, not a port. It reproduces ExifTool's *values* (raw, converted and printed), checked by
differential tests against Perl ExifTool. It does not reproduce ExifTool's execution model, its internal data
structures, or its output formats, except through the CLI's ExifTool-style output.

It is general-purpose. It knows nothing of photo-gallery or any other consumer.

## Blocking decision: the licence

Settled before anything else, because it decides who can use the engine.

- ExifTool is distributed under the same terms as Perl: GPL-1.0-or-later or Artistic-1.0, at the user's choice.
- The engine's tables are dumped from ExifTool's. Whether that makes the engine a derived work, and under which of the
  two licences it may then be distributed, is a legal question; this design does not presume the answer.
- exiftool-rs, which runs ExifTool's own modules, took GPL-3.0-or-later.
- The consequence for consumers is direct. A GPL engine makes every program that links it GPL when distributed.
  photo-gallery, for one, carries no licence, and its `deny.toml` allows no GPL licence: it could not depend on a GPL
  engine without changing both.

Until this is settled, no dumped data is committed and no consumer plans around the engine.

## Why a reimplementation

exiftool-rs runs ExifTool's own modules (dumped Perl data, Perl table code, ported subs) on a Perl interpreter written
in Rust. Its output is ExifTool's; its cost is ExifTool's design. **Measured** on warm reads (interpreter and modules
already loaded), in instructions per read:

| File | A purpose-built reader (photo-gallery's pg-metadata, six fields) | exiftool-rs, nine tags | exiftool-rs, all tags |
|---|---:|---:|---:|
| camera.jpg | 55.7K | 14.0M | 17.8M |
| sample.jpg | 0.9K | 2.3M | 2.7M |
| ExifTool.jpg | 253K | 31.2M | 35.7M |

Four rounds of optimization in the fork took its write case from 17.45G to 6.79G instructions and its warm reads down
19–26%. The gap that remains is structural:

1. **More work.** ExifTool extracts the raw value of every tag of every directory, running each tag's `RawConv` and
   `Condition`, then filters to what was asked (`SetFoundTags`). `ValueConv` and `PrintConv` are lazy, run only for
   the tags returned. It parses maker notes whole (`ProcessBinaryData`: 28% of a camera.jpg read) and evaluates every
   composite definition (`Composite => 0` alone saves 8–30% per read, **measured**).
2. **Each step costs more.** Tag definitions are Perl hashes looked up by byte strings (about 80–115 instructions per
   lookup, about 30,000 lookups per camera.jpg read); every value is a reference-counted Perl scalar (about 37,000
   allocations per read); recording one tag (`FoundTag`) costs about 12,500 instructions; subs dispatch by name through
   the interpreter (about 420 instructions of bookkeeping per call); conditions and conversions are Perl strings run
   by the interpreter, each eval declaring 11–20 variables.

Both follow from running ExifTool's design, so the engine does not.

## ExifTool by the numbers

**Counted** in ExifTool 13.64's `lib/Image` (225 modules, 320K lines), with regular expressions over the source, so
the figures are approximate:

| Shape | Count | What it means for the engine |
|---|---:|---|
| `PrintConv` | 13,079 | 8,668 are inline hashes (value tables); the rest are strings, references and code |
| `ValueConv` / `RawConv` | 2,421 / 1,124 | Of 2,411 one-line strings (914 distinct), 771 are pure arithmetic |
| `ValueConvInv` / `PrintConvInv` | 1,138 / 1,101 | Writing's inverses |
| `Condition` | 2,222 | Of 2,146 one-line ones, 1,023 match a strict typed shape (Make/Model pattern, byte prefix, count, format) |
| `$$self{Make}` / `{Model}` | 840 | Conditions and conversions that depend on other tags |
| `$$valPt` | 497 | Byte sniffing of the value itself |
| Sub references (`\&`) / anonymous subs | 1,764 / 177 | Code, not data |
| `Process*` subs / `ProcessProc` | 488 / 157 | Processing procedures: the bulk of ExifTool's real code |
| `DATAMEMBER` / variable formats | 258 / 127 | Binary tables whose layout depends on earlier values |
| `Priority` / `Avoid` / `PRIORITY_DIR` | 334 / 106 / 18 | Which of several same-named tags wins |
| `AddTagToTable` | 104 | Tags created at run time |
| `Hook` | 54 | Code run while a binary table is read |

The largest modules are Nikon (14.2K lines), TagLookup (14.2K, writing's index), Canon (12.0K), Sony (11.8K),
NikonCustom (11.4K), and the French language module (11.3K; `Lang/` has 18 language modules besides English).

The tables convert mechanically; the code does not. Hand-ported code is the largest part of the work: hundreds of
distinct expressions, more than 1,900 subs, and 488 processing procedures.

## Principles

- **Ask, then read, but resolve like ExifTool.** The caller names tags, groups, or everything. A planner turns that
  into the directories to parse, including every directory that can hold a requested name (so the winner is chosen
  from all candidates) and every directory a condition or conversion depends on (Make, Model, serial numbers). Maker
  notes, thumbnails, ICC profiles and video sample tables are parsed only when something asked for needs them.
- **Own the bytes read, decode late.** Values borrow from buffers the result owns, and the typed meaning is decoded
  when asked for.
- **Numbers, not strings, for what is static.** Tag definitions, tables and fixed groups are numeric identifiers
  with static name tables. What ExifTool creates at run time (below) has a dynamic form.
- **No interpreter.** Conditions and conversions are Rust: generated where their shape allows, ported by hand where
  not, never evaluated from text.
- **Data in, code out.** ExifTool's tables enter as reviewed data files, generated into Rust by a build step.
  Generated code is never edited by hand; hand ports live in their own modules.
- **Bounded and safe.** No panic, no unbounded loop or allocation on any input; limits on depth, sizes and counts;
  every parser fuzzed. Memory per file is bounded by that file's metadata.
- **No unsafe.** `#![forbid(unsafe_code)]` in every crate. Memory-mapped sources are therefore left to the caller:
  a consumer that maps a file passes the slice (see Byte sources).

## Repository and crates

A workspace of its own:

| Part | Role |
|---|---|
| `schema-data/` (not a crate) | The tag data, one file per ExifTool module, checked in and reviewed |
| `schema-ids` (not a crate) | The identifier registry: every definition's stable identifier (see API stability) |
| `dump/` (Perl) | Dumps ExifTool's tables from a loaded Perl ExifTool into `schema-data/` |
| `*-schema-gen` | Reads `schema-data/` and `schema-ids`, writes the generated Rust |
| `*` (the library) | Containers, decoders, generated tables, conversions, composites, reader, writer |
| `*-cli` | A command-line tool, with ExifTool-style output |
| `*-difftest` | The differential harness against Perl ExifTool |
| `fuzz/` | cargo-fuzz targets, one per container and decoder |

`*` is the crate's name, still to choose. It must not be mistaken for ExifTool itself.

The library depends on no consumer's crates. Its public types are its own.

## Architecture

### Byte sources

A `Source` trait: reads by offset and length into a buffer, with a known total length. Implementations for a slice
(which also serves a memory map the caller made) and for a file read by range. Parsers ask for what they need: a
video's box headers are read without its media, and a JPEG's thumbnail only when asked for.

The result owns what was read: the blocks fetched from the source (a slice source's bytes are borrowed instead, at no
cost). Values borrow from those blocks, so a result does not outlive its bytes and needs no copy per value.

### Containers

One module per container: JPEG segments, TIFF/IFD, PNG chunks, ISOBMFF (MP4, QuickTime, HEIF, AVIF, CR3, JXL), RIFF
(WebP, AVI, WAV), EBML (Matroska), GIF, PDF, and the long tail. Each is an iterator over its blocks yielding kinds,
offsets and lengths, never decoded data. Designed fresh for these requirements; existing readers' handling of odd
files (photo-gallery's pg-container and pg-metadata, ExifTool's own) carries over as test cases.

### Directory decoders

- One generic IFD decoder: EXIF, GPS, Interop, IFD1, DNG, and the maker notes laid out as IFDs.
- One generic binary-table decoder. Entries have an offset and a format; the layout may be sequential, with an
  entry's offset or count taken from an earlier entry's value (`DATAMEMBER`, variable formats), and a table may carry
  hooks (`Hook`). The decoder supports all three.
- A streaming XMP/RDF reader, IPTC (IIM), ICC, Photoshop IRB.
- The processing procedures that are not generic (ExifTool's 488 `Process*` subs, by family): offset bases,
  encryption (Nikon, Sony), vendor framing, container-specific walks. Each is a hand port with its own tests.

### The schema

Generated from `schema-data/`:

- Per tag definition: identifier, table, ID within the table, name, format, count, groups (all families),
  writability, priority and avoid flags.
- Its conversions (raw, value, print, and their inverses): a value table (a generated `match`, or a perfect-hash map
  for large ones), a typed formula (`Scale(1, 10)`, `Log2Exposure`, `Ascii`), or a named Rust function.
- Its condition: a typed predicate (`MakeMatches(..)`, `ModelIn(..)`, `CountIs(n)`, `FormatIs(..)`,
  `BytesStartWith(..)`, combined with `And`, `Or`, `Not`), or a named Rust function. Each condition lists the tags it
  reads, so the planner can fetch them.
- Table selection (one maker-note layout per model family) is a `match` over predicates, decided once per directory.
- Language tables (descriptions and printed values in ExifTool's 18 other languages) are generated with the rest.

### Tags, names and groups

Three levels, because ExifTool's identity of a tag is a name and a group, not a definition:

- **Definition** (`TagDef`): one entry of one table, such as IFD0's 0x0112. Static, numeric.
- **Name** (`TagName`): what callers ask for, such as `Orientation`, which IFD0, XMP-tiff and maker notes all define.
  A query by name returns the winner by ExifTool's priority rules, or every instance.
- **Group**: families 0 to 7 as ExifTool defines them. Fixed groups are static; dynamic ones (XMP namespace prefixes
  not in the tables, sub-document numbers, copy numbers) carry their name.

ExifTool also creates tags at run time (`AddTagToTable`: unknown XMP properties, unknown EXIF and maker-note entries
with the unknown option). These get a dynamic definition carrying the table, the ID and a generated name, the same way.

### Semantic types

The decoded meaning of a value is a type: `DateTime` (with an optional offset and subseconds), `Orientation`,
`Rational`, `GpsCoordinate`, `Rating`, `LensSpec`, and so on. Printed text comes from these, through the generated
print conversions.

### Composites

A dependency graph built at compile time: each composite lists the tags it requires and desires. A composite is
computed only when asked for, from tags that were read.

### Results

Per file: the owned blocks and an arena of entries, each with its definition, its group, its sub-document, its raw
value (borrowed), and its converted and printed values on demand. Duplicates are kept, in file order. Lookup by name
(winner or all), by definition, and by group. Results are `Send`; a reader is shareable across threads; each file's
reading is independent, so a caller parallelizes across files.

```rust
let reader = Reader::new().want([names::ORIENTATION, names::DATE_TIME_ORIGINAL, names::RATING]);
let meta = reader.read(&source)?;
let orientation: Option<Orientation> = meta.winner(names::ORIENTATION).and_then(|e| e.decode());
for e in meta.iter() {
    println!("{} {} = {}", e.group(1), e.name(), e.print());
}
```

### Errors and warnings

Errors only for what stops reading (an unreadable source, a container too damaged to walk). Everything else is a
warning: a typed enum with the container, the directory and the offset it refers to. Damaged directories are skipped
with a warning; a file always yields what could be read.

### Text

Character sets as the standards and ExifTool's options define them: EXIF's ASCII and its common UTF-8 misuse, IPTC's
coded character set, XMP's UTF-8/16/32, QuickTime's language-tagged strings. Decoded to Rust strings with the
replacement policy stated per field; the raw bytes stay available.

### Writing

Separate from reading. The editing model is patches per directory; the container is rewritten with offsets
recomputed and unknown data preserved, into a sink the caller supplies. Atomic replacement of a file is a helper on
top, not the core. Maker-note offset fixups are vendor-specific and explicit, never guessed. Formats are added to
writing only with their round-trip and differential tests.

### CLI

ExifTool-style output (`-j`, `-s`, `-G`, `-n`, `-lang`) over the engine's results, for anyone who wants
ExifTool-like text. Printed values themselves are core, not a CLI feature.

## ExifTool's knowledge

The tables convert mechanically; the code is ported by hand.

1. **Dump.** A Perl script loads every ExifTool module with Perl ExifTool 13.64, lets each finish building its tables
   (`AddTagToTable`, `AddCompositeTags`, tables built in loops), and only then writes them to `schema-data/`: one
   record per tag definition, with its fields and value tables. Strings (conditions, conversions) are written as
   their Perl source. A code reference is written as its sub's full name (`B::svref_2object`), or, for an anonymous
   sub, as its deparsed source (`B::Deparse`) and its file and line.
2. **Classify.** The generator maps each condition and conversion to a typed form by its shape. Unmatched ones go to a
   list of hand ports, each named by its source text's hash, so the same expression is ported once wherever it
   appears. The counts above give the scale: about half of the one-line conditions and a third of the one-line
   conversions match the simplest shapes; a richer classifier will match more, and the first dump measures how many.
3. **Hand ports.** Expressions, subs and processing procedures become Rust functions in modules of their own, each
   with tests against ExifTool's results. The generator refers to them by name and fails the build on an unmapped one.
4. **New releases.** Run the dump with the new release; review the diff of `schema-data/`; port what the generator
   cannot classify. The schema records the ExifTool version it came from.

## Correctness

- **Differential tests** against Perl ExifTool 13.64: raw (`-n`) and printed values, compared by name, group and
  sub-document, typed where the engine's type is known. On ExifTool's `t/images`, on files the tests generate, and on
  a public corpus whose licences allow redistribution (for example raw.pixls.us). Every difference is fixed or
  accepted with a recorded reason. Zero unexplained differences is each phase's bar. Tags outside a phase's scope
  (Composite before phase 4, maker notes before phase 2) are excluded by group, explicitly.
- **exiftool-rs as a faster stand-in** for Perl only after it is re-based on 13.64 (it ports 13.60) and shown to
  agree with Perl on the corpus.
- **Fuzzing** of every container and decoder, with the limits above, in CI.
- **Round trips** for writing: write, read back, compare; and Perl ExifTool reads the written file the same.

## Performance

No expected figure is promised. A purpose-built reader's cost (pg-metadata: 0.9K to 253K instructions for six fields)
says the engine can be one to two orders of magnitude cheaper than exiftool-rs, but full extraction with maker notes
has not been built anywhere to measure. Phase 1 measures it and sets the budgets:

- gungraun (callgrind) benches per container, per directory decoder and per full read, from the first phase;
- budgets per bench, set from phase 1's measurements; a regression beyond a budget fails CI.

## API stability

Semantic versioning. Identifiers are stable across releases through `schema-ids`: the generator assigns an identifier
to each new definition and never reuses one. A definition that ExifTool renames is recognized by its table and ID
and keeps its identifier, gaining an alias; a definition removed from ExifTool is kept as deprecated for one major
version. The generated schema's types are public; the generator's are not.

## Phases

Each phase ends with its differential tests at zero unexplained differences, its fuzz targets clean in CI, and its
benches within budget.

0. **The licence** (above).
1. **Core.** The repository and crates; the dump, the identifier registry and the generator; byte sources;
   containers JPEG, TIFF, PNG, GIF and WebP; EXIF, XMP and IPTC with their conversions and languages; reading only;
   the differential harness; the CLI's basic output; the first benches and their budgets.
2. **Maker notes**, by how common they are: Canon, Nikon, Sony, Fujifilm, OM System/Olympus, Panasonic, Apple.
3. **Video and modern containers:** ISOBMFF (MP4, QuickTime, HEIF, AVIF, CR3, JXL), Matroska.
4. **Composites**, as the dependency graph.
5. **Writing**, for phase 1's formats, HEIF and DNG.
6. **The long tail of formats**, in the order the corpus shows them.

## Consumers

Planned only once the licence is settled.

- **photo-gallery** would call the engine through an adapter in pg-metadata that maps the engine's semantic types to
  pg-model's, reading from the buffer the import already holds. Only if the engine's licence is compatible with
  photo-gallery's: today photo-gallery has none and `deny.toml` allows no GPL licence. Once the engine covers the
  import's fields with zero differences, pg-metadata's readers would be retired and their tests moved to the engine's
  suite as regression cases; photo-gallery's own writers (pg-codec's rewriters) stay until phase 5.
- **exiftool-rs** is not a consumer; it is a possible stand-in oracle (above).

## Open questions

- The licence (blocking, above).
- The crate's name.
- The schema-data file format (line-oriented text so diffs review well, versus a structured format such as RON);
  decided with the first dump.
- Which public corpus, and how it is fetched for CI (size, caching, licences per file).
