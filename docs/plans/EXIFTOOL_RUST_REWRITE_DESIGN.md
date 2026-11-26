# A standalone Rust reimplementation of ExifTool

> **Status (2026-11-26).** Design; the engine is not built. Reviewed adversarially five times; the first dump of
> ExifTool's tables and the classifier census have run (appendix) and are committed in photo-gallery's
> `eval/exiftool-dump/`, with the licence deferred (phase 0). The engine is a standalone crate in a repository of its
> own; this file and the dump move there when the repository is made. It replaces the port plans (`EXIFTOOL_PORT_PLAN.md`, `EXIFTOOL_PORT_DESIGN.md`, removed), which
> ported ExifTool's Perl one-for-one into photo-gallery.
>
> The reference is Perl ExifTool 13.64. Figures are **measured** (callgrind in photo-gallery's container, against the
> exiftool-rs fork at commit 9d78ca4), **dumped** (by the first dump), **counted** (regular expressions over
> ExifTool 13.64's `lib/Image`, so approximate), or **estimate**.

## Goal

A Rust library, and a command-line tool on top of it, that reads and writes the metadata of every format ExifTool
handles, with ExifTool's knowledge of tags and vendor quirks, at the cost of a purpose-built reader.

It is a reimplementation, not a port. It reproduces ExifTool's *values* (raw, converted and printed), checked by
differential tests against Perl ExifTool. It does not reproduce ExifTool's execution model or its internal data
structures. It is general-purpose: it knows nothing of photo-gallery or any other consumer.

### Non-goals

- **User-provided Perl.** Everything in ExifTool that runs the user's Perl code is out of scope, with no substitute
  planned: config files (user-defined tags and composites, `%Image::ExifTool::UserDefined`), `-if` conditions, `-p`
  format files with expressions, `${tag;expr}` expressions in tag copying, `-api Filter`, and the other options that
  take Perl. The engine runs only the knowledge that ships with it.
- **ExifTool's output text byte for byte**, except through the CLI's ExifTool-style output.
- **ExifTool's walk of a file**: `-htmlDump` and the verbose dumps (`-v`) show ExifTool's own traversal, which the
  engine does not reproduce.

## Phase 0: the licence

Settled before anything else, because it decides who can use the engine.

- ExifTool is distributed under the same terms as Perl: GPL-1.0-or-later or Artistic-1.0, at the user's choice.
- The engine's tables are dumped from ExifTool's. Whether that makes the engine a derived work, and under which of the
  two licences it may then be distributed, is a legal question; this design does not presume the answer.
- exiftool-rs, which runs ExifTool's own modules, took GPL-3.0-or-later.
- A GPL engine makes every program that links it GPL when distributed. photo-gallery, for one, carries no licence,
  and its `deny.toml` allows neither GPL nor Artistic: it could not depend on the engine under either of ExifTool's
  licences without changing both.

Until this is settled, no consumer plans around the engine and nothing is published. The first dump is committed in
photo-gallery's `eval/exiftool-dump/` meanwhile: on 2026-11-26 the licence was deferred, not settled, so the
question stands for that data as for the engine.

## Why a reimplementation

exiftool-rs runs ExifTool's own modules (dumped Perl data, Perl table code, ported subs) on a Perl interpreter
written in Rust. Its output is ExifTool's; its cost is ExifTool's design. **Measured** on warm reads (interpreter and
modules already loaded), in instructions per read:

| File         | A purpose-built reader (photo-gallery's pg-metadata, six fields) | exiftool-rs, nine tags | exiftool-rs, all tags |
| ------------ | ---------------------------------------------------------------: | ---------------------: | --------------------: |
| camera.jpg   |                                                            55.7K |                  14.0M |                 17.8M |
| sample.jpg   |                                                             0.9K |                   2.3M |                  2.7M |
| ExifTool.jpg |                                                             253K |                  31.2M |                 35.7M |

Four rounds of optimization in the fork took its write case from 17.45G to 6.79G instructions and its warm reads
down 19–26%. The gap that remains is structural:

1. **More work.** ExifTool extracts the raw value of every tag of every directory, running each tag's `Condition`
   and `RawConv`, then filters to what was asked (`SetFoundTags`); `ValueConv` and `PrintConv` are lazy, run only for
   the tags returned. It parses maker notes whole (`ProcessBinaryData`: 28% of a camera.jpg read) and evaluates every
   composite definition (`Composite => 0` alone saves 8–30% per read, **measured**).
2. **Each step costs more.** Tag definitions are Perl hashes looked up by byte strings (about 80–115 instructions
   per lookup, about 30,000 lookups per camera.jpg read); every value is a reference-counted Perl scalar (about
   37,000 allocations per read); recording one tag (`FoundTag`) costs about 12,500 instructions; subs dispatch by
   name through the interpreter (about 420 instructions of bookkeeping per call); conditions and conversions are Perl
   strings run by the interpreter, each eval declaring 11–20 variables.

Both follow from running ExifTool's design, so the engine does not.

## What ExifTool contains

ExifTool 13.64's `lib/Image` is 225 modules and 320K lines. The largest are Nikon (14.2K lines), TagLookup (14.2K,
writing's index), Canon (12.0K), Sony (11.8K), NikonCustom (11.4K) and the French language module (11.3K; `Lang/`
has 18 language modules besides English).

| What                                                        |                                                     Count | What it means for the engine                         |
| ----------------------------------------------------------- | --------------------------------------------------------: | ---------------------------------------------------- |
| Tables / tag definitions / distinct names                   |                      1,532 / 36,222 / 23,177 (**dumped**) | The schema's size                                    |
| Flattened XMP structure fields                              |                   2,702 of those definitions (**dumped**) | Made at run time from structure definitions          |
| Tag IDs with a list of definitions                          |                                          377 (**dumped**) | One ID, several definitions chosen by condition      |
| Value tables shared between definitions                     |                                          448 (**dumped**) | Written once, referenced                             |
| Value tables with an `OTHER` handler / `BITMASK`            |                                    453 / 120 (**dumped**) | Code inside a value table                            |
| `PrintConv`: hash / string / code / list                    | 5,661 / 3,474 / 141 / 74 (**dumped**; 3 more are `undef`) | Printed values: most of ExifTool's knowledge         |
| `ValueConv`: string / code / hash / list                    |                         2,976 / 112 / 16 / 8 (**dumped**) |                                                      |
| `RawConv`: string / code                                    |                                   1,300 / 45 (**dumped**) | Run while reading, often storing state               |
| Inverse conversions: string / code / list                   |         2,931 / 133 / 26 (**dumped**; 3 more are `undef`) | Writing                                              |
| `Condition` / `Hook` strings                                |                                   2,299 / 47 (**dumped**) | Hooks: code run while a binary table is read         |
| Translations                                                |                                       19,907 (**dumped**) | Descriptions and printed values in 18 languages      |
| Named subs / anonymous subs referenced from tables          |                                    300 / 177 (**dumped**) | Code, not data                                       |
| Sub definitions; `Process*` subs; `ProcessProc`             |                       1,439; about 500; 157 (**counted**) | The bulk of ExifTool's reading code                  |
| `WriteProc` / `Write*` subs                                 |                                     49 / 72 (**counted**) | Writing's procedures, besides TagLookup              |
| `$$self{Make}` / `{Model}`                                  |                                         840 (**counted**) | Conditions and conversions that depend on other tags |
| Assignments to `$$self{...}`                                |                   638, to 354 distinct keys (**counted**) | State set while reading, read by later conditions    |
| One-line conditions reading state other than Make and Model |                                970 of 2,146 (**counted**) |                                                      |
| One-line `ValueConv` / `PrintConv` reading state            |                                          53 (**counted**) | Read after reading: the final state                  |
| `$$self{OPTIONS}`                                           |                                         110 (**counted**) | Extraction that depends on the options               |
| `DOC_NUM`                                                   |                                         148 (**counted**) | Sub-documents: MPF images, embedded previews, frames |
| `$$valPt`                                                   |                                         497 (**counted**) | Byte sniffing of the value itself                    |
| `DATAMEMBER` / variable formats                             |                                   258 / 127 (**counted**) | Binary tables whose layout depends on earlier values |
| `Priority` / `Avoid` / `PRIORITY_DIR`                       |                              334 / 106 / 18 (**counted**) | Which of several same-named tags wins                |
| `AddTagToTable`                                             |                                         104 (**counted**) | Tags created at run time                             |

The tables convert mechanically, except their `OTHER` handlers. The code does not, and it is the largest part of the
work: the 1,439 subs (about 500 of them processing procedures, 300 referenced from tables), the 177 anonymous subs,
the 453 `OTHER` handlers, and the 1,120 distinct strings the census could not classify (see "ExifTool's knowledge").

## Principles

- **Ask, then read, resolved like ExifTool.** The caller names tags, groups, or everything; the planner skips what
  nothing asked for needs, and changes nothing else (see Planner).
- **Own the bytes read, decode late.** Values borrow from buffers the result owns; the typed meaning and the printed
  text are computed when asked for.
- **Numbers, not strings, for what is static.** Tables, tag definitions and fixed groups are numeric identifiers with
  static name tables. What ExifTool creates at run time has a dynamic form.
- **No interpreter.** Conditions and conversions are Rust: generated where their shape allows, ported by hand where
  not, never evaluated from text.
- **Data in, code out.** ExifTool's tables enter as reviewed data files, generated into Rust by a build step.
  Generated code is never edited by hand; hand ports live in their own modules.
- **ExifTool's semantics where values depend on them**: its state, its order, its priorities, its options.
- **Bounded and safe.** No panic, no unbounded loop or allocation on any input; limits on depth, sizes and counts;
  every parser fuzzed. Memory per file is bounded by that file's metadata.
- **No unsafe.** `#![forbid(unsafe_code)]` in the library, the CLI, the generator and the harness. The fuzz targets
  are exempt: cargo-fuzz's `fuzz_target!` expands to `#[no_mangle]`, which the lint flags. Memory-mapping a file is
  therefore the caller's (see Byte sources).

## Repository

A workspace of its own:

| Part              | Role                                                                                                       |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| `schema-data/`    | The tag data as JSON Lines, one file per ExifTool module, checked in and reviewed                          |
| `schema-ids`      | The identifier registry: every table's and definition's stable identifier (see API stability)              |
| `dump/`           | The Perl script that dumps ExifTool's tables into `schema-data/`, its cross-check and the census           |
| `*-schema-gen`    | Reads `schema-data/` and `schema-ids`, writes the generated Rust                                           |
| `*` (the library) | Sources, containers, decoders, generated tables, conversions, context, planner, composites, reader, writer |
| `*-cli`           | The command-line tool                                                                                      |
| `*-difftest`      | The differential harness against Perl ExifTool                                                             |
| `fuzz/`           | cargo-fuzz targets, one per container and decoder                                                          |

`*` is the crate's name, still to choose; it must not be mistaken for ExifTool itself. The library depends on no
consumer's crates, and its public types are its own.

## Architecture

### Byte sources

A `Source` trait: reads by offset and length into a buffer, with a known total length. Implementations for a slice
(which also serves a memory map the caller made) and for a file read by range. Parsers ask for what they need: a
video's box headers are read without its media, a JPEG's thumbnail only when asked for.

A source may also provide the file's system metadata: name, directory, size, modify, access and inode-change times,
permissions. ExifTool's System group (`ExifTool.pm` 1320–1508) comes from these, not from the bytes. A file source
provides them, a slice source provides what the caller passes, and a System tag a source cannot provide is absent.

The result owns what was read: the blocks fetched from the source (a slice source's bytes are borrowed instead, at
no cost). Values borrow from those blocks, so a result does not outlive its bytes and needs no copy per value.

### Containers

One module per container: JPEG segments, TIFF/IFD, PNG chunks, ISOBMFF (MP4, QuickTime, HEIF, AVIF, CR3, JXL), RIFF
(WebP, AVI, WAV), EBML (Matroska), GIF, PDF, and the long tail. Each is an iterator over its blocks yielding kinds,
offsets and lengths, never decoded data. They are designed fresh for these requirements; existing readers' handling
of odd files (ExifTool's own, photo-gallery's pg-container and pg-metadata) carries over as test cases.

### Directory decoders

- One generic IFD decoder: EXIF, GPS, Interop, IFD1, DNG, and the maker notes laid out as IFDs.
- One generic binary-table decoder. Entries have an offset and a format; the layout may be sequential, with an
  entry's offset or count taken from an earlier entry's value (`DATAMEMBER`, variable formats), and a table may carry
  hooks. The decoder supports all three.
- A streaming XMP/RDF reader, IPTC (IIM), ICC, Photoshop IRB.
- The processing procedures that are not generic (ExifTool's `Process*` subs, by family): offset bases, encryption
  (Nikon, Sony), vendor framing, container-specific walks. Each is a hand port with its own tests.

A directory's base offset, data and start are a decoder's arguments, as ExifTool passes them in `%dirInfo`; they are
not context.

### The schema

Generated from `schema-data/`:

- **Per tag definition**: identifier, table, ID within the table, name, format, count, groups (all families),
  writability, priority and avoid flags.
- **Its conversions** (raw, value, print, and their inverses): a value table (with its bitmask and hex forms), a
  typed formula, a typed shape from the census (formatting, interpolated text, a substitution, a ternary on a
  predicate), or a named Rust function. A value table's `OTHER` handler is a named Rust function.
- **Its condition**: a typed predicate (on context fields and options, on the bytes of the value, on its count,
  format or value; combined with `And`, `Or`, `Not`), or a named Rust function. Each condition lists the tags and the
  context fields it reads.
- **The definitions of one tag ID**, in ExifTool's order; the first whose condition holds is used, as in ExifTool.
- **Table selection** (one maker-note layout per model family): a `match` over predicates, decided once per
  directory.
- **Language tables**: descriptions and printed values in ExifTool's 18 other languages.

The data is emitted as `static` arrays (definitions, names, value tables, sorted for binary search, or perfect-hash
maps for large ones), not as `match` arms, so the compiler handles tens of thousands of entries as data rather than
code. Code is generated only for predicates and formulas. Build time and binary size have budgets (see Performance).

### Tags, names and groups

Three levels, because ExifTool's identity of a tag is a name and a group, not a definition:

- **Definition** (`TagDef`): one entry of one table, such as IFD0's 0x0112. Static, numeric.
- **Name** (`TagName`): what callers ask for, such as `Orientation`, which IFD0, XMP-tiff and maker notes all define.
  A query by name returns the winner by ExifTool's priority rules, or every instance.
- **Group**: families 0 to 8 as ExifTool defines them. Fixed groups are static; dynamic ones carry their name: XMP
  namespace prefixes not in the tables, sub-document numbers, copy numbers, and family 8's alternate files.

**Run-time tags.** ExifTool creates tags while reading (`AddTagToTable`: unknown XMP properties, unknown EXIF and
maker-note entries with the unknown option). These get a dynamic definition carrying the table, the ID and a
generated name.

**Alternate files.** Family 8 is the alternate file a tag came from: with `-fileNUM`, ExifTool reads other files
alongside the main one and tags their tags `File1`, `File2` and so on (`ExifTool.pm` 4596–4603). The engine supports
them, with names given literally or built from tag values, which needs no Perl.

### Context and options

ExifTool's conditions read more than tags: 970 of the one-line conditions read state other than Make and Model, set
by 638 assignments to 354 distinct keys while the file is read. The engine makes that state explicit and typed, and
gives it ExifTool's semantics.

- **One context per file**, as ExifTool keeps one state hash per file, assigned in read order. Its fields: what
  ExifTool's core keeps (file type, TIFF type, the current document number, the current directory's name and path,
  byte order, Make, Model, and the rest the generator extracts), and one generated field per saved data member (a
  binary table's `DATAMEMBER`, or a value a `RawConv` stores, such as FacesDetected or ColorDataVersion), typed by
  its format.
- **No scope ExifTool does not give.** Make and Model are not per document: IFD0's `RawConv` assigns them
  unconditionally (`Exif.pm` 585, 599), so a sub-document's IFD0 overwrites the main document's values and later
  conditions see the new ones. The TIFF type is set once per file (`ExifTool.pm` 4372, 8770) and can be overridden by
  the file type (8820, 8826).
- **Saved and restored only where ExifTool does.** Around each subdirectory, `ProcessDirectory` restores the byte
  order, pops the path, and restores `DIR_NAME`, `Compression` and `SubfileType` (and `INDENT`, which only verbose
  output uses); `Compression` and `SubfileType` are tag values that conditions read. The generator takes, for each
  field, the definitions and procedures that assign it and the points where it is restored from ExifTool's source,
  not from this list.
- **Two reading times.** Conditions and `RawConv` run while the file is read, so they see a field as it stands at
  that point: unset if nothing has set it yet. `ValueConv` and `PrintConv` are lazy and run after reading, so they
  see it as it stands at the end of the file. The engine keeps both: the live context while reading, and the final
  context with the result.
- **Options**: a typed `Options` struct, read by the planner, the context and the conversions: unknown tags,
  duplicates, character sets, embedded extraction (`-ee`), large-file support, missing values, and the rest of
  ExifTool's options that change what is extracted or how. Output-only options belong to the CLI.

Generated predicates name context fields and options, never ExifTool's keys.

### Planner

The planner turns a request (tags, groups, or everything) into the directories to read. It only skips; it never
reorders, because the winner among tags of equal priority depends on the order in which ExifTool finds them.

A directory is skipped only if all three hold:

1. it can hold none of the requested names, so the winner is still chosen from all candidates (the generator lists
   the tables that define each name; XMP is always read, since its run-time tags can take any name);
2. nothing read after it depends on state it sets or overwrites;
3. no needed tag's conversion reads that state. Needed tags are the requested ones and, transitively, the tags the
   requested composites require or desire.

So maker notes, thumbnails, ICC profiles and video sample tables are skipped unless something asked for needs them.
How much this saves depends on how widely names are shared across tables and how much state flows between
directories, so it is measured, not assumed (see Performance). A full read skips nothing.

### Semantic types

The decoded meaning of a value is a type: `DateTime` (with an optional offset and subseconds), `Orientation`,
`Rational`, `GpsCoordinate`, `Rating`, `LensSpec`, and so on. Printed text comes from these, through the generated
print conversions.

### Composites

A dependency graph built at compile time: each composite lists the tags it requires and desires. A composite is
computed only when asked for, from tags that were read, with the final context.

### Results

Per file: the owned blocks, the final context, and an arena of entries, each with its definition, its group, its
sub-document, its raw value (borrowed), and its converted and printed values on demand. Duplicates are kept, in file
order. Lookup by name (winner or all), by definition and by group. Results are `Send`; a reader is shareable across
threads; each file's reading is independent, so a caller parallelizes across files.

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
warning: a typed enum with the container, the directory and the offset it refers to. A damaged directory is skipped
with a warning; a file always yields what could be read.

Warnings cover what was read: a request that skips directories also skips their warnings, so it can report fewer
than a full read of the same file. The API states this.

### Text

Character sets as the standards and ExifTool's options define them: EXIF's ASCII and its common UTF-8 misuse, IPTC's
coded character set, XMP's UTF-8/16/32, QuickTime's language-tagged strings. Decoded to Rust strings with the
replacement policy stated per field; the raw bytes stay available.

### Writing

Separate from reading, and a design of its own: ExifTool's write side is 49 `WriteProc`s, 72 `Write*` subs,
TagLookup's 14.2K lines and the inverse conversions, with semantics of its own (conditional replacement, list values,
shifted values, deleting groups, copying tags between files by group). Phase 5 starts with that design and its
adversarial review. Fixed now:

- the editing model is patches per directory; the container is rewritten with offsets recomputed and unknown data
  preserved, into a sink the caller supplies; atomic replacement of a file is a helper on top, not the core;
- maker-note offset fixups are vendor-specific and explicit, never guessed;
- a format is added to writing only with its round-trip and differential tests.

### CLI

ExifTool-style output over the engine's results (`-j`, `-s`, `-G`, `-n`, `-lang` and the rest), for anyone who wants
ExifTool-like text. Printed values themselves are core, not a CLI feature. Options that take Perl are refused (see
Non-goals). ExifTool's application features, by phase:

| Feature                                                                                                        | Phase                                          |
| -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| Reading: output formats, `-r`, `-ext`, `-fileNUM`, `-a`, `-u`, `-ee`, `-charset` and the other reading options | 1, and each later phase's formats as they land |
| Writing tags, deleting groups, `-tagsFromFile`, `-overwrite_original`                                          | 5                                              |
| Renaming and moving files by tag (`-FileName<…`, `-Directory<…`, `-d` date formats)                            | 5                                              |
| Importing tags from CSV and JSON (`-csv=`, `-j=`)                                                              | 5                                              |
| Geotagging from GPS logs (`-geotag`, `Geotag.pm`)                                                              | 6                                              |
| `-validate`                                                                                                    | 6                                              |
| `-htmlDump`, `-v`                                                                                              | Not planned (see Non-goals)                    |

Every option of ExifTool 13.64's application is listed in the CLI's documentation with its phase or as not planned,
so none is dropped silently.

## ExifTool's knowledge

The tables are dumped and generated; the code is ported by hand.

1. **Dump.** A Perl script loads Perl ExifTool 13.64 with no config file, builds every table with ExifTool's own
   `LoadAllTables()` and `XMP::AddFlattenedTags` (as ExifTool's BuildTagLookup does, so what ExifTool builds at load
   time and at run time is included), and writes `schema-data/` as JSON Lines: one record per table and per tag
   definition; shared value tables once, under ExifTool's variable name when it has one. Strings (conditions,
   conversions) are written as their Perl source. A code reference is written as its sub's full name, or, for an
   anonymous sub, as its deparsed source (`B::Deparse`) with its file and line. The dump runs on one pinned Perl
   version, because deparsed text differs between versions, records it with the ExifTool version, and is
   deterministic.
2. **Classify.** The generator maps each condition and conversion to a typed form by its shape, recognized on the
   whole string, never on part of it. ExifTool evaluates every table string in package `Image::ExifTool` (in
   `GetTagInfo`, `FoundTag` and `GetValue` in `ExifTool.pm`, and in `Writer.pl`), whatever module the table belongs
   to, so a string's text alone identifies what it does: each unmatched text is one hand port, wherever it appears.
3. **Hand ports.** Expressions, subs, `OTHER` handlers and processing procedures become Rust functions in modules of
   their own, each with tests against ExifTool's results. The generator refers to them by name and fails the build on
   an unmapped one.
4. **New releases.** Run the dump with the new release; review the diff of `schema-data/`; port what the generator
   cannot classify. The schema records the ExifTool version it came from.

**The scale, from the census.** The census sorted the dump's 13,027 strings into the shapes the generator accepts.
Conditions are split on `and`, `or`, `not` and parentheses after their regex and string literals are masked, and are
classified only if every atom is a typed one: a regex, comparison or bit test on a context field or option; a regex,
comparison or length test on the value's bytes, or a byte at an offset; a comparison on the count, format or value.
Conversions are classified as identity, constant, arithmetic formula, `sprintf` of a formula, interpolated text
(`"$val mm"`), one substitution, a call of a named sub with plain arguments, a store into the context, or a ternary on
a predicate whose branches are themselves classified.

| Field                  |    Strings | Classified | Hand ports (distinct texts) |
| ---------------------- | ---------: | ---------: | --------------------------: |
| `Condition`            |      2,299 |      97.5% |                     58 (43) |
| `RawConv`              |      1,300 |      69.6% |                   395 (218) |
| `ValueConv`            |      2,976 |      70.5% |                   878 (409) |
| `PrintConv`            |      3,474 |      90.9% |                   315 (177) |
| `RawConvInv`           |         57 |      15.8% |                     48 (26) |
| `ValueConvInv`         |      1,442 |      74.5% |                   367 (145) |
| `PrintConvInv`         |      1,432 |      75.0% |                    358 (79) |
| `Hook` (always ported) |         47 |         0% |                     47 (40) |
| **All**                | **13,027** |  **81.1%** |           **2,466 (1,120)** |

Strings call 170 distinct subs by name (2,806 calls; the most called are `ConvertDateTime`, `GPS::ToDMS`,
`Exif::ConvertFraction`, `Exif::PrintExposureTime` and `InverseDateTime`); each is a hand port the classified calls
refer to. 16 strings use regex features beyond a plain engine (lookaround, backreferences), so predicates need a
backtracking regex engine for those, or a hand port each. What the census leaves is real code: splitting and
rejoining value lists, `pack` and `unpack`, decryption, date arithmetic, conditions with side effects on the context
(`$$self{CoordUnits} = 1`), and the hooks that adjust a binary table's layout by firmware version. A richer
classifier (lists of formulas, chained substitutions) would take some of it.

## Correctness

- **Differential tests** against Perl ExifTool 13.64, run with no config file: every call passes `-config ''`, since
  ExifTool otherwise loads `~/.ExifTool_config` (or the one under `EXIFTOOL_HOME`) by itself. Raw (`-n`) and printed
  values are compared by name, group and sub-document, typed where the engine's type is known, on ExifTool's
  `t/images`, on files the tests generate, and on a public corpus whose licences allow redistribution (for example
  raw.pixls.us). Each file is read under a stated matrix of options (the defaults, unknown tags, duplicates, embedded
  extraction, the character-set options), on both sides. Every difference is fixed or accepted with a recorded
  reason; zero unexplained differences is each phase's bar.
- **Scope of comparison.** Tags outside a phase's scope (Composite before phase 4, maker notes before phase 2) are
  excluded by group, explicitly. Warnings are compared on full reads only; requested-tag reads are compared on their
  tags. System tags are compared only for file sources.
- **exiftool-rs as a faster stand-in** for Perl only after it is re-based on 13.64 (it ports 13.60) and shown to
  agree with Perl on the corpus.
- **Fuzzing** of every container and decoder, with the limits above, in CI.
- **Round trips** for writing: write, read back, compare; and Perl ExifTool reads the written file the same.

## Performance

No figure is promised. A purpose-built reader's cost (pg-metadata: 0.9K to 253K instructions for six fields) says
the engine can be one to two orders of magnitude cheaper than exiftool-rs, but full extraction with maker notes has
not been built anywhere to measure. Phase 1 measures it and sets the budgets:

- gungraun (callgrind) benches per container, per directory decoder and per full read;
- the planner's saving: requested-tag reads against full reads on the same files, with the generator's reports of
  which tables define each name and which context fields cross directories;
- the build: compile time and binary size;
- a budget for each, set from phase 1's measurements; a regression beyond a budget fails CI.

## API stability

Semantic versioning. Identifiers are stable across releases through `schema-ids`: the generator assigns an identifier
to each new table and each new definition, and never reuses one.

Neither can be recognized by name alone. ExifTool renames and splits tables (per-model `CameraInfo` tables, for one).
Within a table, 377 IDs have a list of definitions chosen by condition, ExifTool inserts new variants in the middle of
such lists, and XMP's IDs are property names, which a rename changes. So the generator matches a new release to the
registry by a stated heuristic:

- tables by their package-qualified name, then by the set of their definitions;
- definitions by their table, then the ID, the condition's text, the name and the format, in that order of weight.

An exact match keeps its identifier; a renamed one keeps it and gains an alias; anything ambiguous fails generation
until a reviewer records the match in `schema-ids`. A table or definition removed from ExifTool is kept as deprecated
for one major version. The generated schema's types are public; the generator's are not.

## Phases

Each phase ends with its differential tests at zero unexplained differences, its fuzz targets clean in CI, and its
benches within budget.

0. **The licence** (above).
1. **Core.** The repository and crates; the dump (done once, appendix), the identifier registry and the generator;
   byte sources; the context, options and planner; containers JPEG, TIFF, PNG, GIF and WebP; EXIF, XMP, Photoshop IRB
   (where a JPEG's IPTC lives) and IPTC, with their conversions and languages; JPEG's JFIF, Adobe and MPF segments;
   reading only; the differential harness; the CLI's reading features; the first benches and the build, cost and
   planner budgets. JPEG's other segments (ICC, Ducky, FlashPix and the vendor ones) are excluded by group until they
   are ported.
2. **Maker notes**, by how common they are: Canon, Nikon, Sony, Fujifilm, OM System/Olympus, Panasonic, Apple.
3. **Video and modern containers:** ISOBMFF (MP4, QuickTime, HEIF, AVIF, CR3, JXL), Matroska.
4. **Composites**, as the dependency graph.
5. **Writing**: first its own design and adversarial review, then phase 1's formats, HEIF and DNG, and the CLI's
   writing features.
6. **The long tail**: the remaining formats in the order the corpus shows them, geotagging, `-validate`.

## Consumers

Planned only once the licence is settled.

- **photo-gallery** would call the engine through an adapter in pg-metadata that maps the engine's semantic types to
  pg-model's, reading from the buffer the import already holds; only if the engine's licence is compatible with
  photo-gallery's (today photo-gallery has none, and `deny.toml` allows neither GPL nor Artistic). Once the engine
  covers the import's fields with zero differences, pg-metadata's readers would be retired and their tests moved to
  the engine's suite as regression cases; photo-gallery's own writers (pg-codec's rewriters) stay until phase 5.
- **exiftool-rs** is not a consumer; it is a possible stand-in oracle (see Correctness).

## Open questions

- The licence (phase 0).
- The crate's name.
- Which public corpus, and how it is fetched for CI (size, caching, licences per file).

## Appendix: the first dump

Run on 2026-11-26 in photo-gallery's container (Perl 5.40.1, B::Deparse 1.76, JSON::PP 4.16) against the ExifTool
clone at tag 13.64, with no config file. Everything is in photo-gallery's `eval/exiftool-dump/` (its README says how to
rerun it): `dump.pl` (the dump), `check.py` (the cross-check against `-list`), `census.py` (the census), the census
report `census.txt`, and the output in `out/` (12 MB, 173 files: 154 `.jsonl` files outside `lang/`, which are the modules, `code.jsonl` and `shortcuts.jsonl`; 18 in `lang/`; and `meta.json`). It moves to the engine's `dump/` and
`schema-data/` when its repository is made.

**Output.** One JSON Lines file per module (`Exif.jsonl`, `Canon.jsonl`, ...) holding `table`, `tag` and `shared`
records; `code.jsonl` (named and anonymous subs); `lang/<xx>.jsonl` (translations); `shortcuts.jsonl`; `meta.json`
(ExifTool, Perl and B::Deparse versions, the evaluation package, counts, load warnings). Values keep Perl's
distinctions: a byte string is a JSON string, a string with Perl's UTF-8 flag `{"u":...}`, bytes that are not UTF-8
`{"bytes":HEX}`, a number stored as a number a JSON number, a reference to a table, tag definition, shared value or
sub a typed reference.

**Checks.**

- Three runs give identical output, byte for byte.
- Every tag name `exiftool -list` prints (21,586) is in the dump; the dump has 1,593 more, the SubDirectory and hidden
  entries `-list` leaves out.
- All 177 anonymous subs deparsed; all 19,463 non-ASCII strings are valid UTF-8 and none carries Perl's UTF-8 flag;
  no number needed more precision than its text; no reference cycles; no warnings while loading.
