# ExifTool 13.60 to Rust: architecture design

Status: design, revised 2026-11-12 to the most proper choices (section 9). No Rust written, nothing compiled. A copy is in the repo at `docs/temp/EXIFTOOL_PORT_DESIGN.md`.
Sources: the 14 group inventories (the JSON in the task), the repo at `D:/GitHub/photo-gallery/crates` (checked: crate list, `crates/metadata/src/lib.rs`, `crates/exiftool/src/lib.rs`, `crates/writeback/Cargo.toml:21`), and the installed tree `C:/Program Files/ExifTool/exiftool_files/lib`. The installed `C:/Program Files/ExifTool/exiftool.exe` is the oracle.

## 0. Scope in numbers

- The inventory covers ~249k Perl lines in ~172 modules, read and write. Every module the groups listed is in scope; the user asked that nothing be removed, so dev-only tools are in scope too (phase P14). This revision adds the modules the inventory missed: Other.pm, Unknown.pm, Lytro.pm, File/RandomAccess.pm (gate O2) and the CLI front end (P14b, gate O7).
- Expression strings (Condition, ValueConv, PrintConv, RawConv, their Inv forms, Hook, Validate, WriteCheck, Format-with-expression, Start/Base): the group figures sum to roughly **9,800**. Largest: Nikon ~2,050 (Nikon.pm 1,475, NikonCustom 496), Sony ~1,250, Canon ~1,150, Other-maker-A ~890, Olympus+Pentax ~835, QuickTime ~460, video ~430, docs ~1,000 (Garmin.pm alone 700, a figure that looks like a grep artefact).
- The inventory's own counts are grep-based, mix patterns, and disagree with each other in places (Writer.pl: "3" in one field, "45 eval lines" in its notes; Canon "952" in a total that also says "about 900"). Treat them as an order of magnitude. **Phase P0 replaces them with an exact census from the Perl dumper (section 4).**
- Group Rust LOC estimates (read side) sum to roughly 170k. The phase table in section 7 totals about 205.5k: read phases 174.0k, writers P13a-d 22.0k, P0 3.0k, P14 4.0k, P14b CLI 2.5k (provisional). The 4k gap to the group figure is the rows added in this revision (Lytro, Other.pm, Unknown.pm, the P2 helpers). All figures are design-level, unverified, and exclude generated data (Lang, Charset, EPSG, lens/model tables, TagLookup), which is mechanical. They also exclude the expression interpreter `pg-et-perl` (section 4), which is not estimated until the P0 census exists.

## 1. Crate layout

### 1.1 Naming and placement

Everything lives under `crates/`, prefix `pg-et-*` (ExifTool), so it does not collide with `pg-exiftool` (the subprocess driver that `pg-writeback` uses, `crates/writeback/Cargo.toml:21`). `pg-exiftool` stays as the migration fallback until the write phases (P13) pass; then `pg-writeback` switches over and `pg-exiftool` is retired. `crates/README.md` (modified in the working tree) holds the repo's crate rules; read it before adding crates.

### 1.2 Proposed crates (dependency order top to bottom)

| Crate | Contents | Depends on |
|---|---|---|
| `pg-et-io` | Random-access reader (File::RandomAccess semantics, file and in-memory), byte-order reader/writer (Get*/Set* family, `ByteOrder` value, no global), output sink, `Fixup` (Fixup.pm); line reader with a caller-supplied record separator (`\xff` for the JPEG segment walk, `:\x1a` for MIFF, `\n` for Radiance, `\r\n` for VCard, CR, LF and CRLF for PostScript) | std |
| `pg-et-value` | Perl scalar model (section 2.2), number stringification (`%.15g`), `%g`/`%f`/`%.Nd` formatting, integer-vs-float arithmetic rules, byte-string helpers, UTF-8 classification (`IsUTF8`, ExifTool.pm 4610-4716), HexDump and TimeNow (moved from Writer.pl) | `pg-et-io` |
| `pg-et-charset` | Charset framework and maps (Charset.pm, Charset/*.pm, 17k lines of generated data), Decode/Encode | `pg-et-value` |
| `pg-et-table` | Type definitions only: `TagKey`, `TagDef`, `TagTable`, `Conv`, `FormatSpec`, `Writable`, `ProcessProc`, `WriteProc`, `TableId` (section 3), the `ExprCtx` trait (field and table access for the interpreter; implemented by `Et` in pg-et-core) | `pg-et-value` |
| `pg-et-expr` | Helpers the interpreter's builtins call: sprintf subset, pack/unpack for the templates the inventory uses, bytes-regex wrappers, strftime/timegm/timelocal subset, bitfield helpers, tr-range compiler, time and zone conversions (ConvertTimeSpan, TimeLocal, GetTimeZone, TimeZoneString, ConvertUnixTime, GetUnixTime), ConvertFileSize, ConvertDuration, ConvertBitrate, ShiftTime and DaysInMonth (read side), a generic ReadJSON parser (Import.pm read side) | `pg-et-value`, `regex` |
| `pg-et-perl` | Perl-subset expression interpreter (section 4): lexer, parser (build time for tables, run time for user text), evaluator over `ExprCtx`, Perl value semantics, PCRE2 regex with captures and `\G`, the builtins the census lists. Compiled trees for every table expression are stored in `pg-et-data` | `pg-et-value`, `pg-et-table`, `pg-et-expr`, `pcre2` |
| `pg-et-data` | **Generated.** Magic table, `%fileTypeLookup`, `@fileTypes` order, `%moduleName`, `%mimeType`, Shortcuts, Lang maps, lens/model/camera tables, all tag tables as static data, and the compiled expression tree for every table expression (section 4). Generated by `pg-et-gen` from committed dumps (section 4.3) | `pg-et-table`, `pg-et-perl`, `pg-et-charset` |
| `pg-et-core` | Per-file context `Et`, options, FoundTag store, GetValue/GetInfo/GetTagList/SetFoundTags, ParseArguments, ExtractInfo, ProcessDirectory, HandleTag, GetTagInfo, ProcessBinaryData, composite engine, Warn/Error, file-system helpers (platform-gated), the processor registry (slot table filled by `pg-et` at startup, section 5.4), ImageDataHash, VerboseInfo/VerboseValue/VerboseDump, InsertTagValues and DumpUnknownTrailer (moved from Writer.pl), ConvertDateTime and its Et-dependent parts, LimitLongValues, ValidateImage, HDump, the Validate.pm and HtmlDump.pm read sides | all of the above; `pg-platform` for Windows and Unix file primitives |
| `pg-et-exif` | Exif.pm, GeoTiff.pm, BigTIFF.pm, MakerNotes.pm dispatch, GPS.pm, read side of WriteExif.pl helpers | core |
| `pg-et-jpeg` | JPEG.pm, ProcessJPEG, trailers (Trailer, AFCP, FotoStation, PhotoMechanic trailer, IdentifyTrailer, ProcessTrailers), JPEGDigest, Stim, Scalado, MPF, FotoStation.pm (read; single home), Unknown.pm | core, exif, meta |
| `pg-et-meta` | XMP.pm, XMP2.pl, XMPStruct.pl (read side), IPTC.pm, ICC_Profile.pm, Photoshop.pm, MWG.pm, Microsoft.pm, APP12.pm, MIE.pm, PLUS.pm, PhotoMechanic.pm tables | core, exif, charset |
| `pg-et-raster` | PNG, MNG, GIF, BMP, ICO, PCX, PPM, PICT, DPX, FITS, MIFF, Radiance, PGF, BPG, FLIF, XISF, Other.pm (PFM), Lytro.pm | core, meta |
| `pg-et-raster-heavy` | Jpeg2000 (JP2/JXL/JUMBF/C2PA), FlashPix (OLE2), PhotoCD, OpenEXR, PSP, GIMP, WPG, LIF, DjVu, BZZ | core, meta |
| `pg-et-video` | RIFF, Matroska, ASF, MXF, MPEG, M2TS, H264, MISB, DV, Real, WTV, Audible, ISO, MOI | core, meta, exif |
| `pg-et-audio` | ID3, FLAC, Ogg/Vorbis/Opus/Theora, APE, WavPack, AIFF, AAC, DSF, MPC, Flash (SWF/FLV) | core, video (ProcessBitStream), meta |
| `pg-et-protobuf` | Protobuf.pm (wire-format decoder; UV/IV/NV duality as Perl; P8a) | core |
| `pg-et-quicktime` | QuickTime.pm, QuickTimeStream.pl, GoPro.pm, LigoGPS.pm | core, exif, meta, protobuf (maker tables via `pg-et-data`, maker processors via the registry, section 5.4; no maker crate dependency) |
| `pg-et-doc` | PDF (+AES), PostScript, ZIP, OOXML, iWork, CaptureOne, RTF, HTML, VCard, Text, Font, PLIST, JSON, CBOR, DarwinCore, InDesign, Torrent | core, meta, charset |
| `pg-et-misc` | 7Z, RSRC, EXE, LNK, TNEF, PCAP, Palm, MacOS, DICOM, Garmin, ITC, GM | core, meta, doc (ZIP for CaptureOne etc.) |
| `pg-et-raw` | CanonRaw read, MinoltaRaw, PanasonicRaw, Kyocera, SigmaRaw (X3F), PhaseOne, Rawzor, DNG, Leaf, Red, ZISRAW, MRC | core, exif, maker crates |
| `pg-et-maker-canon` | Canon.pm, CanonCustom.pm, CanonVRD.pm | core, exif |
| `pg-et-maker-nikon` | Nikon.pm, NikonCustom.pm, NikonCapture.pm, NikonSettings.pm | core, exif, meta |
| `pg-et-maker-sony` | Sony.pm, SonyIDC.pm | core, exif, maker-minolta (in maker-a) |
| `pg-et-maker-olympus-pentax` | Olympus.pm, Pentax.pm (and HP/PrintIM consumers) | core, exif, meta |
| `pg-et-maker-a` | FujiFilm, Panasonic, Kodak, Minolta, Samsung, Sigma, Ricoh, Casio, Sanyo | core, exif, meta |
| `pg-et-maker-b` | Apple, Google, DJI, FLIR, Qualcomm, Parrot, Nintendo, GE, HP, JVC, Reconyx, Motorola, PrintIM, Kandao, InfiRay | core, exif, meta, protobuf |
| `pg-et-geo` | Geotag.pm, Geolocation.pm (+ asset), Plot.pm, Shortcuts.pm | core, exif, quicktime |
| `pg-et-write-core` | Writer.pl core (SetNewValue, WriteInfo, WriteDirectory, WriteJPEG, WriteBinaryData, WriteTIFF, value Set* packers, Rationalize, EncodeBits, file time, rename helpers), WriteExif.pl, DoProcessTIFF write branch, trailer write, Fixup use, Import.pm (CSV/JSON in). P13a | core, exif, jpeg |
| `pg-et-write-meta` | WriteXMP.pl, WriteIPTC.pl, WritePhotoshop.pl, WriteICC_Profile, XMPStruct write side. P13b | write-core, meta |
| `pg-et-write-containers` | WriteQuickTime.pl, WriteRIFF.pl, WritePNG.pl, WritePostScript.pl, WritePDF.pl, FLIF, JP2/JXL box writers, GIF write, MIE write. P13c | write-core, quicktime, raster, raster-heavy, video, doc |
| `pg-et-write-makers` | WriteCanonRaw.pl and the maker write procs (Canon, CanonCustom, CanonVRD, Nikon, Sony, PhaseOne, Kodak, Samsung, FujiFilm RAF, PanasonicRaw, MRW, Sanyo, Olympus/Pentax table writes). P13d | write-core, exif, raw, maker crates |
| `pg-et` | Facade: builds the static processor registry, `ImageInfo`/`GetInfo`-style public API, the CLI driver (once its source is inventoried, section 8) | everything |
| `pg-et-gen` | Dev-only generator: reads `spec/*.json` (dumper output), writes `pg-et-data` source. Not a dependency of shipped code | none (dev) |
| `pg-et-testkit` | Synthetic fixture builder (TIFF, JPEG, PNG, MP4, RIFF, etc.), oracle runner, mismatch reporter. Dev/test only | `pg-exiftool` (driver), `pg-et` |

### 1.3 Reuse of existing crates

- **`pg-metadata` (`crates/metadata`)**: do not reuse as the reader. It reads five things (rating, dates, orientation, frame count, XMP packet) by design ("only the tags used here", its lib.rs header). ExifTool needs every table entry, conversion and write path. It can *inform* the EXIF reader in `pg-et-exif` and its `container.rs` may be reused as a seed for `pg-et-io`; check both for the exact byte semantics they already test.
- **`pg-container`**: candidate reuse for container walking (it exports `range_in`, which `pg-metadata` uses). Check whether it validates ranges the way ExifTool's RAF reads do. Do not depend on it for anything that must match ExifTool's error and warning behaviour.
- **`pg-model`**: `Format`, `Orientation`, `Rating`, `parse_iso_date` are the app's domain types. Do not put ExifTool's value model there; keep the port free of app types. The app can convert at its boundary.
- **`pg-platform`**: reuse for Windows file open and path helpers where it already wraps the Win32 calls. ExifTool's own Windows paths (ExifTool.pm 4718-5079: long paths via GetFullPathNameW, CreateFileW, FILETIME/DST fix) need `windows-sys` calls; check what `pg-platform/src/native.rs` already has before writing new ones.
- **`pg-exiftool`**: keep as is. It is the stay-open driver for the oracle harness (`pg-et-testkit`), so the harness does not reimplement process handling. Its own comments already cover the argument-file encoding issues the harness will hit.
- **`pg-codec`, `pg-video`, `pg-thumbs`, `pg-render`**: image decode and thumbnails, not metadata. No reuse.

### 1.4 Rules for the new crates

- No global mutable state anywhere (section 2.4). Process-wide ExifTool state (`%static_vars`, `$ENV{TZ}`, Lang selection, Win32 caches) lives in an explicit context or behind `OnceLock`.
- The library crates deny `print_stdout`/`print_stderr`, like the existing crates do (`crates/metadata/src/lib.rs` header). Warnings are tags, not prints.
- Build natively in PowerShell (user memory: native build, not WSL). Batch the compile at the end of each phase, not between steps (user memory: compile at the end).
- No new branch: commit on `main` only when asked.
- Dependencies: family crates depend on `pg-et-core` and on the non-family crates in section 1.2. A family crate depends on another family crate only where the table says so, and only while the graph stays acyclic (pg-et-raw and pg-et-write-makers on the maker crates; pg-et-maker-sony on pg-et-maker-a; pg-et-geo on pg-et-quicktime). Every other cross-family reference (sub-table processors, trailer processors, MakerNotes dispatch, expression helpers owned by another family) goes through the registry in pg-et-core (section 5.4), filled by pg-et. No reader crate depends on a write crate. The table was checked for cycles: none.
- Licence and attribution of the port are open (decision D14). No crate is published until D14 is decided.

## 2. Core value model

### 2.1 Tags, groups, priority and file order

ExifTool's central store is `FOUND_TAGS` plus the `VALUE`, `TAG_INFO`, `TAG_EXTRA`, `FILE_ORDER`, `PRIORITY` and `DUPL_TAG` members (ExifTool.pm FoundTag 9498-9678; inventory core-B). In Rust:

- `TagKey` (in `pg-et-table`): the Perl hash key as written in the table. An enum `Int(u32)`, `Frac { int: u32, part: u8 }` (for `1.1`, `10.1`, `33.10`: the decimal text must stay distinct, so `Frac` keeps the source text), or `Str(&'static str)` (for `'0 0'`, `'49474.10'`, `'0xdeadbeef'` written as a string). Perl's stringification of numeric keys is part of the identity (Sony.pm and Canon.pm both depend on this; inventory pg Sony risks).
- `TagDef` / `TagInfo`: the static definition (section 3). A tag instance found in a file is an `Entry` (below).
- `Entry`: `{ key: TagKey, name: Cow<'static, str>, groups: GroupSet, priority: i32, raw: Val, info: &'static TagDef or overlay id, file_order: u32, doc_num: u32, copy: u32 }`. The store is an ordered `Vec<Entry>` in insertion order plus a `HashMap<name, SmallVec<index>>` for the `VALUE{name}` lookups. This matches what FoundTag does (VALUE by name, FILE_ORDER for sorting, duplicates kept as `Name (N)`), without the refs-as-keys aliasing the inventory warns about (`LIST_TAGS` and `COMP_KEYS` write through references at ExifTool.pm 9567, 9629-9631, 9667).
- The duplicate rule (promote priority, keep last-seen at equal priority, `Name (N)` for the rest) is copied from FoundTag and SetFoundTags (5325-5559). Inventory risk: the dedupe pass keeps the last-seen tag among equal priority and the list order comes from Perl's randomized hash iteration (lines 5371, 5373, 5443, 5446). **Decision needed (section 9, D6):** fix the iteration order to `FILE_ORDER`, and check the winner against the oracle on duplicate-name fixtures.
- Groups: family 0-8 names are computed from the table's `GROUPS` defaults plus per-entry `G0..G8` overrides in `TAG_EXTRA` (GetGroup 3811). The memoisation into the shared group hash (`GotGroups`) becomes an immutable precomputed `GroupSet` per `TagDef` at build time, plus a per-entry override. No hidden write into tables.
- Priority: `Priority` attribute on TagDef, plus the `PRIORITY_DIR` / `LOW_PRIORITY_DIR` state that ProcessDirectory sets (9096-9151). Store on the Entry at FoundTag time.

### 2.2 Values

Perl scalars are IV, UV, NV or PV with lazy conversion. The port needs one enum that preserves what ExifTool observes:

```rust
enum Val { Undef, Int(i64), UInt(u64), Float(f64), Bytes(Arc<[u8]>) /* binary and string */,
           List(Vec<Val>), Ref(Box<Val>) /* SCALAR-ref results, e.g. \$val */, Struct(...) }
```

Rules that have to be exact (each has a source in the inventory):

- Stringification of floats is `%.15g` (inventory exif-tiff risks; Protobuf.pm UV/NV duality). Write `fmt_g15`.
- Integer arithmetic in expressions: Perl promotes to NV on overflow; `use integer` appears in BZZ.pm and must be honoured by the expression fn that the module compiles to. Protobuf.pm needs UV/IV/NV duality for 64-bit values; use `UInt(u64)` explicitly and promote as Perl does.
- String versus number comparisons that the code relies on (for example `$$et{Compression} eq '34892'`, `$lensType eq 65535`, DNG version `"$1.$2" <= 1.7`, Nikon `$val eq -1`): each comparison site in the generated expression keeps the Perl comparison kind (`eq` vs `==`), not a coerced one.
- `Ref(Box<Val>)` exists because RawConv and ValueConv return scalar refs for binary payloads (`\$val`, inventory Exif/XMP/PNG notes). A `Ref` is printed as the placeholder text by Printable (6544-6569) and is not a string.

### 2.3 Per-file context

`Et` (in `pg-et-core`) is the per-file context and replaces `$self`. The inventory counts dozens of `$$self{...}` members, some read by later tags (`LensType`, `FocalUnits`, `CanonFirmVer`, `Make`, `Model`, `BitM`, `PentaxDate`, `NewNikonSerialKey`, `TIFF_TYPE`, `FILE_TYPE`, and ~40 more across the groups). Design:

- Typed fields for the members the core touches (the Init reset list, 4326-4383; the new() list, 2348; FILE_TYPE, PATH, RAF, FILENAME, EXIF_DATA, FILE_ORDER, PRIORITY, TAG_INFO, VALUE, DOC_NUM, DOC_COUNT, INDENT, CHANGED, DEL_GROUP, NEW_VALUE, EDIT_DIRS, ADD_DIRS, PROCESSED, BASE, etc.). Persistent versus per-file: the inventory lists exactly which lowercase-free members persist (`Duplicates`, `OPTIONS`, `FILE_SEQUENCE`, `PRIORITY_DIR`).
- Format-specific members that expressions read or write go in a typed sub-struct per family crate (`CanonState`, `NikonState`, `SonyState`, ...), owned by that family's processor. Cross-family reads (Make, Model) stay in core.
- The generator (section 4) scans every expression for `$$self{NAME}` and `$self->{NAME}` and fails the build if NAME is neither a typed core field nor declared in the owning family state. This replaces the untyped hash, and it catches the inventory's "state read by a later tag" hazards at build time.
- `$$et{...}` (the ExifTool object, not a family) and `$$self{...}` are the same object in Perl. The port has one `Et` and one set of names; the generator uses the same registry for both.

### 2.4 Process-wide state

Each of these becomes an explicit field or a documented exception:

| Perl global | Rust |
|---|---|
| `%static_vars` (KeepUTCTime, SystemTimeRes, OverrideFileDescription) | field on `Et`, set by Options |
| `$ENV{TZ}` side effect of TimeZone (2669) | `Et` field (a zone id or fixed offset set by the TimeZone option). Local offsets are computed from it (Windows TZ API for the zone rules, applied as an explicit offset). The port never writes the process environment. A fixed zone for an oracle run is set by the test harness on the child process it spawns (section 8.3), never inside the library |
| `%UserDefined`, `%userLens`, Lang tables | loaded at `Et` construction, immutable after |
| `$evalWarning` | captured per expression: each evaluation returns its value and the warnings it raised (D16, chosen: reproduce) |
| `%allTables`, `@tableOrder` | registry in `pg-et-data`, built once (`OnceLock`) |
| `%compositeID`, `%writeTypes` | built once from data |
| Win32 `$k32*` caches (5079-era) | `OnceLock` per function |
| `$currentByteOrder`, `$swapBytes`, `$swapWords` | `ByteOrder` value carried by the reader (inventory risk, ExifTool.pm 5984-6207 and everywhere it is saved and restored) |
| Decrypt state `$ci0, $cj0, $ck0, $decryptStart` (Nikon.pm) | a `NikonCipher` struct owned by the Nikon processor |
| `@grps` (Composite RawConv, Olympus, SonyIDC) | a return value from the composite evaluator |

### 2.5 Warnings and errors

`Warn` and `Error` add tags (5617-5662), and processing continues. Rust does not use `Err` for them. `Result<_, Fatal>` is reserved for the cases Perl aborts (die, unhandled read failure reported as Error and stop). The mapping to tags, minor-error flags, `WAS_WARNED` counting and `[xN]` suffixes (ExtractInfo 3190-era, Warn 5617-5662) is one function in `pg-et-core`, with table tests for each flag combination (inventory core-B note).

### 2.6 Context-sensitive subs

Every Perl sub that returns different values in list and scalar context gets two named Rust functions, one per context (for example `get_value_list` and `get_value_scalar`). The inventory names these: GetValue, GetFileType, GetGroup, GetTagID, ParseArguments, GroupMatches, SetFoundTags, ReadValue, RebuildTIFF, ReadFLIFHeader. Each call site uses the function for the context its Perl call site had.

## 3. Tag tables

Every Perl hash in the inventory becomes a static Rust value. The target shape is one `TagTable` per table, one `TagDef` per tag ID, generated.

```rust
pub struct TagTable {
    pub id: TableId,                // stable index, from the registry
    pub name: &'static str,         // "Image::ExifTool::Exif::Main"
    pub groups: GroupDefaults,      // GROUPS
    pub process: ProcessProc,       // PROCESS_PROC enum (fn pointer behind it)
    pub write: Option<WriteProc>,   // WRITE_PROC enum
    pub check: Option<CheckProc>,
    pub flags: TableFlags,          // VARS: ALLOW_REPROCESS, NO_LOOKUP, ID_FMT, SORT_PROC, LONG_TAGS, ...
    pub first_entry: i64, pub format: Option<FormatSpec>, pub priority: i32,
    pub entries: &'static [(TagKey, TagEntry)],   // sorted by key at build time
}
pub enum TagEntry { Single(TagDef), Alternatives(&'static [TagDef]) /* first Condition wins */ }
pub struct TagDef {
    pub name: &'static str, pub writable: Writable, pub groups: GroupOverride,
    pub format: FormatSpec, pub count: CountSpec, pub mask: Option<(u32, u8)>,
    pub value_conv: Conv, pub value_conv_inv: Conv, pub print_conv: Conv, pub print_conv_inv: Conv,
    pub raw_conv: Option<&'static CompiledExpr>, pub raw_conv_inv: Option<&'static CompiledExpr>,
    pub condition: Option<&'static CompiledExpr>, pub validate: Option<&'static CompiledExpr>, pub write_check: Option<&'static CompiledExpr>, pub del_check: Option<&'static CompiledExpr>,
    pub subdirectory: Option<SubDir>, pub hook: Option<&'static CompiledExpr>, pub flags: TagFlags, pub notes: Option<&'static str>,
}
pub enum Conv {
    None,
    Expr(&'static CompiledExpr),          // compiled expression tree (section 4)
    Table(&'static [(&'static str, &'static str)]),   // PrintConv hash; keys stored as the text Perl compares
    Bitmask { table: &'static [(u32, &'static str)], other: Option<&'static CompiledExpr> },
    Other(&'static CompiledExpr),
    Relist(...), ArrayOfPerValue(...)  // Relist/REPEAT, list conversions (GetValue 3431-3741)
}
```

How each Perl construct maps:

- **Hash literal of tags**: generated `entries` slice, sorted, duplicate keys resolved last-wins at generation time (inventory risks: GeoTiff 3072 key 2177 and keys 26774-26777; GoPro MUID 293/456; Sigma 0x929 three times; Olympus/Pentax 33024 four times; NikonCustom SettingsD5100 23.1; Nikon BracketingInfo D500/D810). The generator emits a warning list; each one is a quirk record (section 9, D8).
- **Arrays of conditional alternatives** (`[ {Condition=>..}, {...} ]`): `TagEntry::Alternatives`; first Condition true wins, as in GetTagInfo (9205-9267).
- **Shared references** (`%allGroupsExifTool`, `%geoInfo`, `%binaryDataAttrs`, `%cropConv`, `%rawCropConv`, `%convFloat2`, `%wbTypeInfo`, Minolta afStatusInfo spread into ~90 Sony entries): Rust `static` items, referenced by address, or expanded by the generator. Spread hashes (`{ %geoInfo, Notes => ...}`) are expanded at generation time (the merged result is what Perl produces at load, so the expansion is exact).
- **Code refs** (`\&Sub`, `sub {...}`): Expressions written as `sub {...}` are compiled trees (section 4). `\&Sub` references to processors are `fn` pointers to the ported processor (section 5.2).
- **Format and count expressions** (`int16u[$val{3}]`, `undef[$size-0x7d3]`, `undef[3*$$self{PaletteColors}]`): `FormatSpec::Dynamic(&'static CompiledExpr)`, evaluated by the reader before the value is read. Hook (`$varSize += ...`) mutates the running offset of the binary walker; `Hook` returns the new size.
- **Load-time table mutation** (Canon/Leaf/Panasonic/Protobuf expansion, Sony lens merge from Minolta, Qualcomm generated names, GE, Composite registration via AddCompositeTags at load, `@MakerNotes::Main` copied into DNG AdobeData): executed by the generator, not at run time. Table order is the generator's dependency order, which must match the Perl load order (inventory risk, Leaf/DNG/PanasonicRaw).
- **Run-time table mutation** (AddTagToTable for unknown tags under -u/-ee; ProcessKeys adding ItemList entries; TextualInfo in Kodak; JVC_Text_*; MISB/Audible/Canon custom unknowns; FLIR MeasInfo): a per-run `TagOverlay` in `Et`, consulted before the static table. Order of insertion is preserved (inventory: output order). The overlay is the only mutable table state.
- **`GetTagTable('Image::ExifTool::...')` string lookups and SubDirectory `TagTable` names**: `TableId` indices assigned by the generator. A missing id is a build error, not a run-time `eval`.
- **Notes q{} blocks** (documentation only): kept as `notes`, not used at run time. `TagNames.pod` is regenerated from them if P14 wants it.
- **Writable / WRITE_PROC / CHECK_PROC**: `writable` and `write`/`check` fields; read-only builds still carry them (tags are visible to `-listwritable`-style output and to the writer).

The `pg-et-data` crate is large and generated. Expected size: the inventory's table entries are ~75% of the Perl lines in the maker modules. Generated code must compile in a reasonable time: split the data into one module per source file (`data/exif_main.rs`, `data/nikon_main.rs`, ...) and keep each `static` a slice literal.

## 4. Expression interpreter (decisions D1, D3, D7, D9, D16)

### 4.1 What is interpreted

Perl expression text, wherever ExifTool keeps it:

- table fields: Condition, ValueConv and ValueConvInv, PrintConv and PrintConvInv, RawConv and RawConvInv, Hook, Validate, WriteCheck, DelCheck, Format and the Start, Base and IsOffset expressions, and `sub {...}` blocks;
- run-time text supplied by the user: the Filter option (D7), and `.ExifTool_config` with its piped `-` form (D3);
- the subs those expressions call, when they take the `$et` or `$self` object and only read or write its state and their arguments (for example `Image::ExifTool::Canon::CanonEv` and `Exif::PrintExposureTime`).

Processors and writers are not interpreted. `Process*`, `Write*` and the container readers and writers are ported as Rust (sections 5.2 and 6). The line between the two is set by the census: a sub that opens files, does I/O, or recurses into a processor is a processor and is ported natively.

### 4.2 Decision: an interpreter (D1)

The Perl text is the source of truth. `pg-et-perl` parses it:

- at build time for every table expression. The generator stores the compiled tree in `pg-et-data` (one tree per expression id, section 4.3);
- at run time for user text (Filter, the config file).

Trees are evaluated against `Et` through the `ExprCtx` trait, with the Perl semantics the expressions depend on:

- precedence and associativity as Perl parses them, including the quirks the census finds (for example `cond or $et->Warn(...), last;`, `$pos += 0x20 > length(...)`, and the `or`/`and` precedence at QuickTimeStream.pl 2495). These are reproduced, not cleaned up;
- the scalar model of section 2.2: IV, UV, NV and PV duality, `%.15g` stringification, and `use integer` where a module sets it;
- regex with captures (`$1` to `$n`), `pos` and `\G`, backreferences, and `\Q...\E`, all through PCRE2 (section 4.4);
- `sprintf`, `pack` and `unpack` with the templates the census lists, `substr`, `index`, `lc`, `uc`, `join`, `split`, and `sort` with its block form;
- list and scalar context, through the paired functions of section 2.6;
- `local` and string `eval` only where the census shows them (Format and Start strings);
- `die` and `warn` routed to tags, as HandleTag and GetTagInfo do (D16);
- `$$self{NAME}` and `$$et{NAME}` as typed field access (section 2.3). An unknown name is a build error for table expressions and a reported error for user text.

Why an interpreter, and not the cheapest option: one implementation of Perl's rules is checked by every expression it runs. A hand translation of about 9,800 expressions creates 9,800 places for those rules to go wrong, each needing its own check. A bug in the interpreter shows up in every expression that hits it, the oracle reports each one, and one fix clears them all. The interpreter also covers the config file (D3), the Filter option (D7) and NoWarning (D7), which a translation could not. Embedding Perl (option c) stays rejected: it would run ExifTool, not port it.

The interpreter is the critical path. It is not sized in this document. Its estimate comes from the P0 census, and it is re-estimated when P1 exits.

### 4.3 Census, parser check and tests

- P0 writes `dump_tables.pl`, run with the perl named by D15 (`/usr/bin/perl`, 5.42.2 on this machine, which loads `Image::ExifTool` 13.60 and has `B::Deparse`). It walks every table in `%allTables` and every expression slot in section 3, and emits the text, owner, table, tag key, and the `B::Deparse` output of each code ref. Output is committed under `crates/pg-et-data/spec/`. The census replaces every grep count in the inventory.
- Parser check: the tree `pg-et-perl` builds for each expression must match the `B::Deparse` tree for it, after Deparse's normalisation (explicit parentheses, constant folding). Each difference is either a parser bug, or a normalisation the parser has to reproduce. All are resolved before P1 exits.
- Evaluator check: every expression id (`module:table:key:slot:n`) has at least one oracle case from the synthetic fixtures (section 8.2) that drives it to a value. An id with no such case is listed as untested, with the reason, in the phase report.
- The generator emits one tree per expression id. It fails the build if an id has no tree, if a `$$self` name is neither a typed field nor declared in its family state (section 2.3), or if a table id is missing (section 3).

### 4.4 Regex dialect (D9)

Every regex site uses PCRE2 (the `pcre2` crate, with its bundled C build). The inventory's patterns need Perl features that the Rust `regex` crate does not have:

- lookahead and lookbehind: `riff(?!RIFF)` (FLAC), `(?!\0...)` (OlympusTags2), `(?!Exif\0\0)` (Jpeg2000), the IsFloat lookaheads;
- backreferences: M2TS RMC `$13` to `$15`, and HTML `\1`;
- named captures, `\G` at a scan offset (the `pos()` scanners), and `\Q...\E` quoting of runtime-built patterns.

Hand-written scanners are not used for regex. A pattern stays a pattern so that the dialect is the same across all sites. PCRE2 still differs from Perl in corner cases (empty-match iteration, `$` before a final newline, case folding of non-ASCII text under `(?i)`). Each site is checked against the oracle, and each difference found is recorded in the quirk register (D8).

### 4.5 Residual gap (D3)

The config file is executable Perl, so its code runs through the interpreter. User code may use Perl constructs that ExifTool's own sources never use. The interpreter supports the census set. A construct outside that set is reported as a load error when the config loads. It is never silently ignored, and it is added to the interpreter when a user needs it. This is the one gap in the port, and it is recorded here so it is visible.

## 5. Dispatch

### 5.1 File type and module (ExtractInfo)

ExifTool picks the file type from the extension (`%fileTypeLookup`, 351 entries, 231-587, with alias strings and candidate lists resolved in GetFileType 4243-4244), then from the magic number (`%magicNumber`, 108 entries, 929-1048, tested against the first `$testLen = 1024` bytes), then loops over candidate types calling `Process<Type>` through `%moduleName` (66 entries, 854-921; `''` means Image::ExifTool, `0` means recognised but unsupported). FastScan levels 0-5 gate read, processing and `%processType` (inventory still-images, core-A). The detection order is `@fileTypes` (198-205, 115 names) and must be kept exactly.

Rust:

- `pg-et-data` emits `FILE_TYPES: &[FileTypeRow]` in `@fileTypes` order, `EXT_LOOKUP` (sorted slice, binary search, aliases resolved at generation time where Perl resolves them at run time; keep the run-time alias step as a fallback for user types), `MODULE_OF: &[(&str, ModuleId)]`, and `MAGIC: &[(FileType, MagicMatcher)]`.
- `MagicMatcher` is a PCRE2 byte pattern (section 4.4). Check each of the 108 against PCRE2 in byte mode: `(?i)` (VCard, HTML), lazy `.{0,191}?` (M2TS), bounded `.{n}` up to 512 (PICT, MRC), `\xff` classes. No lookaround or backreference appears in the magic table (inventory core-A). Build-time test: every pattern compiles and matches its fixture.
- `PROCESSORS: &[(ModuleId, &str, ProcessFn)]` is the static registry replacing `require` plus `&$func` (ExtractInfo, ProcessTrailers, AUTOLOAD). Built by the `pg-et` facade from the family crates (section 5.4); a phase-gated entry is `None` until its phase lands, and the build fails if a file type in `@fileTypes` has no processor and no explicit "unsupported" marker.

### 5.2 Table processors and writers

`PROCESS_PROC` and `WRITE_PROC` values in the inventory are code refs (`\&ProcessBinaryData`, `\&ProcessJPEG_HDR`, `\&Image::ExifTool::XMP::ProcessXMP`, `\&WriteExif`, `\&WriteQuickTime`, ...) or string names resolved at run time (`"Image::ExifTool::CanonVRD::WriteCanonDR4"`). Rust: `enum ProcessProc { BinaryData, Exif, Tiff, Jpeg, Xmp, Ps, QuickTime, ... }` and `enum WriteProc { ... }`, both generated from the dumper's list of distinct targets, each mapping to one fn. One `match` in `pg-et-core::dispatch`. No string dispatch.

### 5.3 Trailers

ProcessTrailers (7043-7191) and IdentifyTrailer (6990-7030) use a DirName-to-module mapping built inside the loop in Perl. In Rust: a static `TRAILERS: &[(&str, ProcessFn)]` in the order the loop tries them, with the Perl loop order preserved (inventory: `Process*` names, AFCP/FotoStation/PhotoMechanic/CanonVRD/MIE/Samsung/Insta360/Nikon/Vivo/OnePlus/Google). Fixup state (`AddFixup`) is an explicit value passed through the loop. Trailer processors that live in family crates (Samsung, Insta360, Nikon, Vivo, OnePlus, Google, MIE, CanonVRD, AFCP, FotoStation, PhotoMechanic) are reached through the core registry (section 5.4); `pg-et-jpeg` does not depend on the maker crates.

### 5.4 Phase gating

Table references cross families (QuickTime refers to Canon/Nikon/Olympus/Pentax/Samsung/Kodak/Sony sub-tables; DNG refers to CanonRaw, Minolta, Sony, FujiFilm, Kodak, Panasonic, Leaf). Until the referenced family lands, the registry marks the table `Unavailable(phase)`. A fixture that needs it is excluded from that phase's oracle run and listed in the phase report. The table is never silently skipped in output. The registry is a slot table in `pg-et-core`, indexed by `ProcessProc` and `TableId`; `pg-et` fills the slots at startup. A family crate reaches a table it does not own through its slot, never by naming the family crate that owns it. An empty slot is `Unavailable(phase)`.

## 6. Write path

### 6.1 What the writer does

- `SetNewValue` (Writer.pl 312-1237) builds a linked list of NEW_VALUE records with DelCheck, WriteAlso (`goto WriteAlso`, lines 944 and 976), RawConvInv/ValueConvInv/PrintConvInv and WriteCheck (inventory Writer.pl). `WriteInfo` then drives the write per output file.
- `WriteDirectory` dispatches to the table's WRITE_PROC. `WriteJPEG` reads segments with `$/ = "\xff"`, uses labelled loops, and buffers output with fixups. `WriteTIFF`/`WriteExif` rebuild each IFD, merge existing and new entries, sort, and copy deferred image blocks (WriteExif.pl 550-2747). `WriteBinaryData` writes fixed-layout tables and honours DataMember and VarFormatData.
- Container writers rewrite their own format: QuickTime (`WriteQuickTime` recursive atom rewriter, 921-2226, ~30 control states), RIFF/WebP (two-pass with in-memory buffers), PNG (two-pass, text chunks before IDAT, CRC), GIF (comments and application extensions before the first non-extension block), PDF (incremental update, xref table or stream, encryption), PostScript (DSC comments, padded XMP packet), FLIF, JP2/JXL boxes, FujiFilm RAF, MRW, CR2/CRW.

### 6.2 Design

- The writer produces into an owned `Vec<u8>` (or a file sink for streaming formats) through the `pg-et-io` output sink. `Fixup` (Fixup.pm, 366 lines, reused by WriteExif, ProcessTrailers, MakerNote fixups, AFCP, Sony SR2) is a value, not a shared mutable graph: a tree of `(start, shift, pointers)` built during write, applied once at the end. The inventory's shared-mutable-fixup risk (WriteQuickTime `$$boxPos`, `ChunkOffset` arrays of arrays) becomes explicit vectors of `ChunkOffset { ptr, len, base, id }`.
- `goto` becomes an explicit state machine (WriteExif has 7 gotos; SetNewValue WriteAlso; InDesign `goto DONE` becomes a `Result` return). Every such rewrite gets a test derived from the Perl control flow, not from the output.
- Output-by-mutation (`$_[1]` in EncodeFileName, SetResourceName, Sanyo FixOffsets, ToFloat, IO_TAG_LIST) becomes returned values or `&mut` parameters, one per site. The inventory lists each.
- Maker write procs (Canon WriteCanon, Nikon ProcessNikonEncrypted re-encryption, Sony WriteEnciphered with DoubleCipher, PhaseOne WritePhaseOne, Kodak WriteKodakIFD, MRW WriteMRW, Sanyo FixOffsets) live in their family crate and are wired in P13d.

### 6.3 Byte-exact requirements

These are what "byte-exact" must cover. Each is a test with an oracle output:

1. Append order of IFD entries, value buffers, subdirectories and deferred image blocks (WriteExif; reverse tag order, last IFD first for ImageData copy).
2. Offset fixups: shift values, 0xfeedfeed placeholders patched later (WriteExif 2157 check).
3. Padding: PhotoMechanic to 0x800 unless `Compact NoPadding`; Photoshop IRB padding; XMP packet padding and the extended-XMP split with MD5 GUID (LimitXMPSize, WriteXMP.pl); CRW record lengths; IPTC extended length (0x8000 flag).
4. DNG OriginalRaw and FLIF/PNG/JP2/Brotli payloads: byte-equal once the builds are matched (D5, O4); decoded payloads are a second check.
5. IPTC record ordering in DoWriteIPTC; mandatory-tag rules; PDF xref rebuild and object numbering; PostScript `%%+` continuation at 254 characters.
6. Time-dependent output: NewGUID/NewUUID and `TimeNow`. Oracle runs pin the clock or exclude these tags; the port uses the same source (Perl drand48 for `srand`, see D10).

### 6.4 Relation to pg-writeback

`pg-writeback` currently calls `pg-exiftool`. The cutover to `pg-et-write` happens after P13 passes its byte-exact gate for the formats the app writes (JPEG, TIFF/DNG, PNG, WebP/RIFF, MP4 if used, XMP and IPTC on those). Check which formats `pg-writeback` actually writes before scheduling P13 (open question O6).

## 7. Phases

Each phase lists its modules, est_rust_loc (design level; excludes generated data), prerequisites, and the **done** test against the oracle. A phase is not done until its gate passes on the corpus in section 8 and every mismatch is either fixed or recorded as a quirk (section 9, D8). Phases can run in parallel where the prerequisites allow.

Phase conventions:

- Oracle gate: `exiftool.exe -j -G1 -n` (numeric) and `-j -G1` (print conversions on), with the volatile tags excluded (section 8.3), compared field by field.
- Each phase batch-compiles once at the end (user memory).

Phase DAG (checked for cycles; each line is a phase and its hard prerequisites, with P4's soft completion by P5a and P5b noted):

- P0: none (entry gate O1, D15). P1: P0, O2. P2: P1. P3: P2. P4: P3.
- P5a: P2, P3. P5b: P5a. P6a: P2, P5a, P5b. P6b: P5a, P5b, P6a (D14 for BZZ).
- P7a: P2, P3, P5b. P7b: P5b, P7a.
- P8a: P2, P3, P5a, P5b, P6b, P7b. P8b: P8a, P5b, P7b.
- P9a: P3, P5a. P9b: P5a, P5b, P9a.
- P11b: P2, P3, P5a, P5b. P11d: P2, P3, P5b. P11a: P2, P3, P5a, P8a. P11e: P2, P3, P4, P5a, P5b, P8a. P11c: P3, P5a, P5b, P11e. P11f: P2, P3, P5b, P8a, P8b.
- P10: P3, P11a, P11c, P11d, P11e. P12: P2, P5b, P8a.
- P13a: P1-P4. P13b: P13a, P5a, P5b. P13c: P13a, P5a, P5b, P6a, P6b, P7a, P8a, P9a. P13d: P13a, P10, P11a-P11e.
- P14: P2-P13. P14b: the O7 inventory, then P14.

**P0 Harness and census.** Modules: `dump_tables.pl` (dev, Perl), `pg-et-gen` skeleton, `pg-et-testkit` (fixture builder, oracle runner on `pg-exiftool`, mismatch reporter), expression census. est 3,000. Prereq: none. Entry gate: O1 answered and the perl binary named (D15: `/usr/bin/perl`, checked 2026-11-12). Exit items: the two-run oracle determinism check (O3, before D6 is decided), the corpus inventory (section 8.2 item 6), and the expression census that the interpreter's coverage target needs (section 4.3). Done: the dumper runs over all ~172 modules and produces committed spec files; the census gives exact expression counts per module; the runner reports identity (exiftool against itself) with zero diffs on 50 real files; the synthetic builder produces one TIFF, one JPEG and one MP4 that exiftool reads with the expected tags.

**P1 Value model, I/O, options, charset data.** Modules: `pg-et-io` (RAF, byte order, sink, Fixup), `pg-et-value`, `pg-et-table`, `pg-et-perl` (the expression interpreter, section 4; est TBD after the P0 census), `pg-et-charset` (Charset.pm and Charset/* data), `pg-et-data` skeleton (options registry from `@availableOptions` 1112-1217, `%charsetName`, `%specialTags`, `%langName`), Options/ClearOptions (2419-2715), Init members (4326-4383), DataAccess accessors (5984-6207), ReadValue (6211-6333), Decode/Encode/DecodeBits (6345-6408), Printable (6544-6569), IsUTF8 (4610-4716), Get64/GetFixed/GetExtended (Writer.pl helpers the reader needs, moved into `pg-et-io`), HexDump and TimeNow (moved from Writer.pl), File::RandomAccess (not inventoried; gate O2, which must exist before P1 starts). Estimate breakdown: inventory rows 1,640 (Options 420, Fixup 220, DataAccess 260, ReadValue 110, Decode/Encode 110, Printable 40, IsUTF8 130, charset 350); value model 600; RandomAccess 700 (assumed until O2); byte order and Get64/GetFixed/GetExtended 350; HexDump and TimeNow 60; typed Init struct 200; table types 250; options registry code 100; est 3,900. Prereq: P0, O2. Done: line-reader tests, one per record separator listed in the pg-et-io row; unit tests for every Get/Set width and both byte orders against `pack`/`unpack` vectors; ReadValue output for every format code against oracle values from a synthetic TIFF with each format; option table matches `exiftool -listoptions` (or the AvailableOptions output).

**P2 Read engine.** Modules: ExtractInfo (2717-3225), file-system tags and Windows path helpers (4718-5079), ParseArguments (5081-5191), GetInfo and GetTagList (3227-3429), GetValue (3431-3741), GetTagID/GetDescription/GetGroup (3743-3968), BuildCompositeTags and GetCompositeTagInfo (3970-4174), CombineInfo/DoneExtract (the part of 4326-4608 after the P1 Init members), GetFileType/CanWrite/CanCreate (4176-4321), SplitFileName etc., ProcessDirectory (9096-9151), GetTagInfo (9205-9267), AddTagToTable (9269-9327), GetTagTable/TagTableKeys (8977-9089), HandleTag (9329-9489), FoundTag (9498-9678), SetFileType (9738-9812), Warn/Error (5617-5662), ExpandShortcuts (5669-5758), SetFoundTags/RemoveTagsFromList/CopyAltInfo (5219-5559), ProcessBinaryData (9927-10225), ExtractBinary (9886-9919), VPrint and verbose helpers (9817-9878), magic and file-type data (`pg-et-data`), AddCompositeTags/SetupTagTable/ExpandFlags (5764-5931), GroupMatches (5219-5262), IsPC/IsFloat/IsInt (5939-5979), Printable/MakeDescription/MakeTagName (6452-6501), DoAutoLoad replaced by the static registry (5565-5598). Added in this revision: time and zone helpers in `pg-et-expr` (ConvertDateTime 6575-6694, with its ShiftTime call; ConvertTimeSpan and TimeLocal 6700-6737; GetTimeZone and TimeZoneString 6743-6777; ConvertUnixTime and GetUnixTime 6785-6846; est 450); ConvertFileSize, ConvertDuration, ConvertBitrate, ConvertFileName, InverseFileName, LimitLongValues (6852-6953; est 150); ValidateImage (6415) and HDump (6959-6982; est 80); VerboseInfo, VerboseValue, VerboseDump, DumpUnknownTrailer, ImageDataHash and InsertTagValues, moved from Writer.pl into `pg-et-core` (est 350); the Validate.pm read side, ValidateRaw, FinishValidate and ValidateOffsetInfo (est 300); the HtmlDump.pm read side, Add, Print and FinishTiffDump (est 400); ReadJSON (Import.pm, into `pg-et-expr`; est 250; moved from P14 because Lytro and QuickTime need it); ShiftTime and DaysInMonth read calls (Shift.pl; est 150). The added rows are estimated from line ranges, not inventory rows (O20). est 9,130 (7,000 + 2,130). Prereq: P1. Done: `-j -G1` output for 500 arbitrary non-format files (text, binary, empty, bogus extension, directories) matches oracle on the FileName/Directory/Size/Modify/Access/Create/Attributes/Permission tags and the `Error`/`Warning` lines; file-type detection matches for all 115 `@fileTypes` entries via synthetic headers. Symlink fixtures are skip-when-unprivileged (Windows developer mode or the symlink privilege). Long-name and non-ASCII-name fixtures are gate O15 (section 10), with their exact names listed there before this gate runs.

**P3 EXIF, TIFF, GeoTIFF, BigTIFF, maker dispatch.** Modules: Exif.pm (7,324 lines: Main, Composite, lookups, ProcessExif, conversion helpers), GeoTiff.pm, BigTIFF.pm, MakerNotes.pm (dispatch and LocateIFD/FixBase), DoProcessTIFF read branch (8592-8971, read part), ProcessTIFF/EXIF/SubTIFF wrappers (8542-8586), read-side helpers from WriteExif.pl (ValidateIFD, GetOffList, ValidateImageData, AddImageDataHash, RebuildMakerNotes, ProcessTiffIFD read), ProcessSubTIFF, ValidateExif, the Validate.pm call site for EXIF (its read side is in P2). est 9,800. Prereq: P2 (including its Validate and HtmlDump read sides). Maker tables outside this phase are `Unavailable` (section 5.4). Done: synthetic TIFF fixture per Exif::Main tag key (generated by the fixture builder from the table; every key gets a fixture with its format and count) matches oracle in print and numeric modes; DNG-like and CR2-like synthetic headers identify the same FileType; GeoTIFF directory tags match for the EPSG table entries; `IFD0`/`ExifIFD`/`GPS`/`InterOp`/`SubIFD` group names match.

**P4 JPEG and trailers.** Modules: JPEG.pm (84 expression strings, 838 lines), ProcessJPEG (7272-8536), JPEG marker tables and DirStart (7197-7265), ProcessTrailers and IdentifyTrailer (6990-7191), JPEGDigest (static 2,320-entry table as generated data), Trailer.pm (Vivo, OnePlus, Google), AFCP.pm, FotoStation.pm (read), Stim.pm, Scalado.pm, MPF.pm (composite RawConv runs ExtractMPImages), PhotoMechanic trailer read, Unknown.pm (%Unknown::Main, the Unknown-tag table and the PrintIM dispatch; est 30). est 3,980 (the inventory rows sum to 3,950, plus Unknown.pm). Prereq: P3 (EXIF), P5 (XMP/IPTC for APP1/APP13 sub-blocks; can start with stubs that return "unsupported" and be completed with P5). Done: JPEG fixtures with every APP0-APP15, COM, SOF, DQT marker and a trailer of each supported kind match oracle; JPEGDigest identical on 100 real JPEGs; MPF composite PreviewImage matches; an unknown-tag fixture with `-u` matches (Unknown.pm).

**P5a XMP family.** Modules: XMP.pm (4,785 lines, read side: namespaces, tokenizer, FoundXMP, GetXMPTagID, AddFlattenedTags, ConvertXMPDate), XMP2.pl (extension namespaces, SEAL), XMPStruct.pl (read side: ConvertStruct, RestoreStruct). est 4,800. Prereq: P2, P3 (ExifTime). Tokenizer: the regex tokenizer relies on `//g` and `pos()`; run it as PCRE2 with `\G` at the scan offset (D9, section 4.4). Done: XMP packets from real files (photos, PDFs with XMP, MP4 with XMP) match oracle; the XMP struct tags (ast/nine/Google-style) flatten identically.

**P5b Other metadata families.** Modules: IPTC.pm (read; 1,308 lines), ICC_Profile.pm (1,462; read), Photoshop.pm (1,292; read), MWG.pm (778; the cross-group Artist mutation at load), Microsoft.pm (1,279), APP12.pm (323), MIE.pm (2,583; zlib via `flate2`, group recursion, 0x80 free-space), PhotoMechanic.pm (read), PLUS.pm (2,622; generated Media Matrix), GPS.pm (642; tables and ToDMS/ToDegrees used by everything after), MIEUnits.pod (377 lines; the unit abbreviation list as generated data for MIE, with a count check). est 9,350 (generated data excluded). Prereq: P5a. Done: IPTC and Photoshop IRB blocks from real JPEG/TIFF/PSD files match oracle; ICC profile tags for sRGB, Adobe RGB and an embedded profile in a PNG match; MWG Artist behaviour matches on a file with Artist in both EXIF and XMP; MIE sample with compressed group matches (decoded values); the MIE unit table count equals the count in MIEUnits.pod.

**P6a Common raster formats.** Modules: PNG.pm (read; 1,735 lines; Compress::Zlib replaced by `flate2`), MNG.pm (tables), GIF.pm (read), BMP.pm, ICO.pm, PCX.pm, PPM.pm (the pos() cursor issue, section 9 D9), PICT.pm (struct DSL, 21 count expressions: run through the P1 interpreter), DPX.pm, FITS.pm, MIFF.pm, Radiance.pm, PGF.pm (re-entrant ExtractInfo, depth limit 10), BPG.pm (ue7 Get), FLIF.pm (read), XISF.pm, Other.pm (ProcessPFM2 and its PrintConv eval string; est 50), Lytro.pm (214 lines; reads JSON through the P2 ReadJSON; est 180). est 5,950. Prereq: P2, P5a (XMP for several), P5b (ICC, Photoshop, IPTC). Done: one real file per format (and synthetic variants for the quirks named in the inventory) matches oracle; PNG text and eXIf/zXIf match; PPM with a comment over 1024 bytes matches. A PFM file matches (Other.pm); a Lytro sample, or a synthetic one built from its table, matches.

**P6b Heavy containers.** Modules: Jpeg2000.pm (1,695; JP2, JXL, JUMBF/C2PA, CBOR/JSON via P5/P9), FlashPix.pm (2,431; OLE2, FAT/mini-FAT, property sets, VT_*, Word FIB, ExtractEmbedded, FPXR), PhotoCD.pm, OpenEXR.pm, PSP.pm, GIMP.pm, WPG.pm, LIF.pm, DjVu.pm plus BZZ.pm (decoder, 471 lines; based on DjVuLibre under the GPL, so BZZ starts only after decision D14). est 7,530. Prereq: P5a, P5b, P6a (PNG cpIp dispatch into FlashPix), and D14 for BZZ. Done: FlashPix sample (.fpx) and an embedded FPXR APP2 match; JP2 and JXL samples match; DjVu ANTa/ANTz match on a sample; OpenEXR multipart sample matches with DOC_NUM.

**P7a Video containers.** Modules: RIFF.pm (2,273; WAV/AVI/WebP read; WriteRIFF deferred to P13c), Matroska.pm (1,289), ASF.pm (901), MXF.pm (3,031; ~2,400 UL entries, generated), MPEG.pm (735), M2TS.pm (1,084; PCAP-style regex backrefs, see D9 and the `$lines[0]` defect), H264.pm (1,149; bitstream Golomb), MISB.pm (494), DV.pm (315), Real.pm (739), WTV.pm (319), Audible.pm (317), ISO.pm (206), MOI.pm (159). est 11,170. Prereq: P2, P3 (EXIF chunk), P5b (ID3 is P7b; M2TS GEOB uses ID3 only for c2pa). QuickTime-embedded callers are `Unavailable` (section 5.4) until P8a; P7a does not depend on P8. Done: a file per container family from the corpus matches oracle; Matroska 0x13b8 and 0x47a behaviour matches the oracle as the quirk record says; H264 SEI and SPS fields on a real MP4 match.

**P7b Audio and Flash.** Modules: ID3.pm (1,775), FLAC.pm (321; ProcessBitStream shared), Ogg.pm (241), Vorbis.pm (255), Opus.pm (98), Theora.pm (145), APE.pm (287), WavPack.pm (144), AIFF.pm (316; 80-bit extended), AAC.pm (177), DSF.pm (138), MPC.pm (156), Flash.pm (749; AMF0, zlib via `flate2`). est 3,820. Prereq: P5b (XMP DecodeBase64, ConvertXMPDate), P7a (RIFF, MPEG). The ID3 genre table is generated data that P8a uses, so it lands here. Done: an MP3 with ID3v1, v2.3 and v2.4, an APE, a FLAC with picture, an Ogg Vorbis with COVERART, a WAV, an AIFF, an SWF and an FLV from the corpus match.

**P8a QuickTime container.** Modules: QuickTime.pm (10,888 lines; ~7,000 are table data including the 2,642-entry genre PrintConv), the atom walker and its hand code, Protobuf.pm (323; `pg-et-protobuf`, consumed by the P11f DJI and Google processors), and the maker sub-table references as `Unavailable` markers (section 5.4). est 6,000 (provisional split of the 11,360 inventory sum for P8a and P8b together; O20). Prereq: P2, P3, P5a, P5b, P6b (Jpeg2000 for HEIC and CR3 metadata), P7b (the ID3 genre table, which is generated data). Done: MP4 with `mvhd`/`tkhd`/`mdhd`/`udta`/`ilst`/`keys`, and an HEIC with iloc/iinf/ipma, match oracle. The Perl precedence quirks at QuickTimeStream.pl are in P8b (D8).

**P8b Timed metadata.** Modules: QuickTimeStream.pl (3,840; 113 expression strings; freeGPS formats; the `ProcessFreeGPS` and `ScanMediaData` `\G` scanners, PCRE2 with `\G` at the scan offset (D9)), GoPro.pm (944), LigoGPS.pm (432; decryption, bit-level), and the ReadJSON consumer for Process_gdat (the parser is in P2). est 5,360 (provisional split, see P8a). Prereq: P8a, P5b, P7b. Done: a GoPro GPMF MP4, a dashcam freeGPS sample and a Nextbase sample match oracle. Known quirks: Perl precedence at QuickTimeStream.pl 2449 and 2495/2517 (D8).

**P9a Documents and text formats.** Modules: PDF.pm (2,538; read; RC4, AES, SHA-2 for Rev6; FlateDecode via `flate2`), AES.pm (501; use a vetted AES primitive, keep the CBC/padding/error behaviour; the IV comes from an OS RNG where ExifTool uses Perl `rand`, recorded in the quirk register D8: encrypted output differs in the IV only), PostScript.pm (793), OOXML.pm (411), ZIP.pm (869; `zip` crate; RAR4/RAR5 handwritten), iWork.pm (230), CaptureOne.pm (235), RTF.pm (390), HTML.pm (583), VCard.pm (457), Text.pm (244), Font.pm (932; WOFF2 Brotli via a Brotli crate), PLIST.pm (548), JSON.pm (202), CBOR.pm (351; the `exp` defect), DarwinCore.pm (404; data). est 7,650. Prereq: P5a, P3. Done: PDF with Info and XMP (plain and encrypted with RC4 and AES-128/256), an OOXML docx, an ODT, a ZIP with members, an RTF, an HTML with meta, a vCard, an EPS, a TTF and a WOFF2 sample, a binary plist, and a C2PA CBOR block match oracle.

**P9b Archives, executables and platform formats.** Modules: 7Z.pm (793; LZMA1 via a Rust LZMA crate; `ReadUINT64` typo and `== 0` defect), RSRC.pm (247), EXE.pm (1,589), LNK.pm (1,902), TNEF.pm (487), PCAP.pm (462), Palm.pm (403), MacOS.pm (761; read side; macOS shell-outs are D7-platform), DICOM.pm (8,081; ~7,700 lines of tag and UID data), Garmin.pm (6,678; ~6,000 lines of enum tables), ITC.pm (215), GM.pm (551), Torrent.pm (332), TNEF (above), InDesign.pm (298; read), AFCP (P4). est 8,860 (inventory rows 9,020, less FotoStation's 160, which sits in P4; generated data excluded; the DICOM 900 and Garmin 1100 rows may include generated tables, which the P0 census settles). Prereq: P5a, P5b, P9a. Done: a PE and an ELF sample, a .lnk and a .url file, a 7z with LZMA1 header, a pcapng and a classic pcap, a DICOM file, a Garmin FIT, a Palm PRC/MOBI, a TNEF winmail.dat, a .torrent; all match oracle, with macOS-only tags marked skipped on Windows.

**P10 Raw formats.** Modules: DNG.pm (871; read side), MinoltaRaw.pm (537; MRW read), PanasonicRaw.pm (956; read; PatchRawDataOffset is write side in P13d), KyoceraRaw.pm (173), SigmaRaw.pm (702; X3F read), PhaseOne.pm (763; read), Rawzor.pm (190; bzip2 via a crate; recursive ExtractInfo ReEntry), Leaf.pm (517), Red.pm (336), ZISRAW.pm (242; ShortenTagNames order), MRC.pm (341; 113 expression strings, mostly bitmask conditions). CanonRaw read is in P11a. est 4,830. Prereq: P3, P11a (CanonRaw tables), P11c (Sony SR2Private for DNG), P11d (Olympus/Pentax as referenced), P11e (FujiFilm RAF, Kodak IFD, Minolta). Done: a DNG (from the corpus or exiftool-written), an MRW, a Kyocera RAW, an X3F, a PhaseOne IIQ-like synthetic, a RWZ, an R3D, a CZI, an MRC/CCP4 sample match oracle.

**P11a Canon family.** Modules: Canon.pm (11,643; ~950 expression strings, ~4,500 table lines), CanonCustom.pm (2,900; 72 strings; Functions2 holds most), CanonVRD.pm (2,306; 126 strings; read side for trailer and DR4; the write side is P13d), CanonRaw.pm (923; read side, CRW), and its MakerNotes-building helpers used by the read path only when the MakerNotes option is set (InitMakerNotes/BuildMakerNotes/SaveMakerNotes live in WriteCanonRaw.pl; port them into this phase's read path, not as writers). est 11,500. Prereq: P3, P5a (XMP block in VRD), P2, P8a (the CR3 read path runs through the QuickTime container). Done: CR2 (synthetic and corpus), CRW and CR3 samples match (CR3 through P8a, with the Canon table here); CameraInfo tables for the models present in the corpus match; lens name via PrintLensID matches on 20 real lens-ID values; FirmwareRevision and FileNumber composites match.

**P11b Nikon family.** Modules: Nikon.pm (14,494; 1,475 expression strings; ~600-entry lens table; Decrypt/SerialKey cipher; PrescanExif; ProcessNikonEncrypted; ProcessNikonMOV/AVI/App/NKA), NikonCustom.pm (11,436; 496 strings; ~2,600 Mask rows), NikonCapture.pm (940; 28 strings; read side), NikonSettings.pm (2,150; 55 strings). est 18,250. Prereq: P3, P5a, P5b, P2. Done: NEF samples from the corpus (D3500-era and Z-series if present) match in full; ShotInfo and LensData decryption matches on a file with known serial; LensID composite matches on 20 lens values.

**P11c Sony family.** Modules: Sony.pm (11,968; 1,230 live expression strings; 2,200 table lines; Decipher/ProcessEnciphered; SRF/SR2 read; PIC; rtmd read; LensSpec decode), SonyIDC.pm (393; 17 strings). est 11,350. Prereq: P3, P5a, P5b, P11e (Minolta tables merged at load). Done: ARW samples (A100-era SR2 and modern ARW) match; enciphered tags decode; FocusDistance and GPS composites match; LensType via sonyLensTypes2 matches on 20 values.

**P11d Olympus and Pentax.** Modules: Olympus.pm (4,469; 195 strings; ~200 lens types, ~330 camera types), Pentax.pm (6,978; 640 strings; ~330 lens codes, ~240 model IDs, 46 fractional keys, the Hook/varSize mechanism, CryptShutterCount, DecodeAFPoints, PrintFilter). est 7,400. Prereq: P3, P5b (GPS helpers), P2. Done: an E-series JPEG, an OM-D/PEN MakerNote, a K-series PEF/DNG, an Optio WP MOV, an AVI from the Pentax family match; PictureMode and Gradation relisted composites match; the 33024 ImageTone case behaves as the quirk record says.

**P11e Maker family A.** Modules: FujiFilm.pm (2,078), Panasonic.pm (3,000; includes ProcessLeicaTrailer regex scanner), Kodak.pm (3,280; 191 strings; TextualInfo runtime AddTagToTable), Minolta.pm (2,960; the shared lens/teleconverter/scene/colour tables Sony uses), Samsung.pm (1,943; Crypt; STMN; trailer), Sigma.pm (911; sigmaLensTypes with three 0x929 keys), Ricoh.pm (1,277), Casio.pm (2,060), Sanyo.pm (440; FixOffsets is write side, its read tables here). est 14,150. Prereq: P3, P5a/b, P2, P4 (JPEG QVCI/Meta/APP5 callers), P8a (QuickTime callers; `Unavailable` until P8a lands). Done: a FujiFilm RAF and a Panasonic RW2 (Leica trailer present in one sample if available), a Kodak DC-series JPEG, a Minolta A100 SR2, a Samsung trailer MP4, a Casio EX-series JPEG, a Sigma X3F-era lens ID, a Ricoh GR JPEG, a Sanyo sample all match.

**P11f Maker family B.** Modules: Apple.pm (412), Google.pm (820; HDRP cipher, gunzip via `flate2`), DJI.pm (1,017; protobuf expansion at load), FLIR.pm (1,669), Qualcomm.pm (1,421; generated names at build time), Parrot.pm (893), Nintendo.pm (128), GE.pm (83; ProcessGE2 forward declaration with no body; recorded in the quirk register D8 as declared, no body; grep the Exif callers before closing; nothing is skipped), HP.pm (263), JVC.pm (130), Reconyx.pm (730; 53 inverse expressions), Motorola.pm (178), PrintIM.pm (125), Kandao.pm (399), InfiRay.pm (229). GoPro is P8b. est 6,255. Prereq: P3, P5b, P8a (Protobuf), P8b (GoPro), P2. Done: an iPhone HEIC with RunTime, a DJI JPEG (Mavic or Osmo), a FLIR JPEG (APP1 with the calibration), a Google Pixel JPEG with HDRP if present (else synthetic cipher vectors from the oracle), a Parrot mett sample, a Reconyx JPEG, a Kandao KVAR, a 3DS MPO, a JVC, a Motorola, a PrintIM block all match.

**P12 Geo.** Modules: Geotag.pm (1,787; GPX, KML, NMEA, IGC, CSV, Google JSON, Winplus, Bramor, metadata fallback), Geolocation.pm (1,060; the 3.3 MB binary as an asset, D11), Plot.pm (733; SVG, Perl drand48 for `srand(141)`, D10), Shortcuts.pm (359; data and LoadShortcuts). LigoGPS is P8b. est 3,000. Prereq: P5b (GPS), P2, P8a (QuickTime coordinate helpers). Done: a GPX track geotag of a JPEG matches (GPS tags and interpolation); Geolocate on 10 coordinates matches the oracle's nearest city; a -plot SVG is byte-identical on a fixed input (colours included if D10 is "reproduce").

**P13a Writer core and EXIF/JPEG/TIFF write** (crate `pg-et-write-core`). Modules: Writer.pl (7,449 lines, less the helpers moved to P1 and P2: HexDump, TimeNow, VerboseInfo, ImageDataHash, DumpUnknownTrailer, InsertTagValues; the rest: SetNewValue, WriteInfo, WriteDirectory, WriteJPEG, WriteBinaryData/CheckBinaryData, WriteTIFF, value Set* packers, Rationalize, EncodeBits, file time, rename helpers), WriteExif.pl (2,747 lines, write side), ExifTool.pm DoProcessTIFF write branch (8828-8970), Trailer write (ProcessTrailers OutFile), Fixup use, Writer-side GetNewValue/IsOverwriting/GetNewTagInfoHash/InitWriteDirs (in ExifTool.pm 1-5200 new-value API as used by writers). est 7,000. Prereq: P1-P4 (P4 trailers), and the helpers moved into P1 and P2 (section 7.1). Done: exiftool.exe-produced output byte-identical for a fixed list of 100 commands on JPEG and TIFF (`-Make=`, `-DateTimeOriginal+=`, `-tagsfromfile`, `-delete`, `-all=`, GPS writes) and identical after reading back.

**P13b Writer metadata families** (crate `pg-et-write-meta`). Modules: WriteXMP.pl (1,673), WriteIPTC.pl (724), WritePhotoshop.pl (270), ICC_Profile write (WriteICC_Profile), XMPStruct write side (CheckStruct, DeleteStruct, AddNewStruct, AddNewTag). est 3,000. Prereq: P13a, P5a, P5b. Done: XMP and IPTC edits on JPEG (Keywords, Rating, Title, GPS fields) and TIFF match oracle byte for byte, including the extended-XMP split at the size limit with a long Description.

**P13c Container writers** (crate `pg-et-write-containers`). Modules: WriteQuickTime.pl (2,315), WriteRIFF.pl (395), WritePNG.pl (414), WritePostScript.pl (700), WritePDF.pl (801; incremental update, xref stream), FLIF write, Jpeg2000 CreateNewBoxes/CreateColorSpec/ProcessBrotli write, GIF write, FujiFilm WriteRAF (P13d or here), MinoltaRaw WriteMRW (P13d), BMP/ICO none, MIE WriteMIEGroup/CheckMIE (inside MIE). est 6,000. Prereq: P13a, P8a (QuickTime read), P6a, P6b, P7a, P9a (per family, section 7.1). Done: MP4 Keys/ilst write and HEIC item edit match oracle (zlib payload compared decoded); WebP EXIF/XMP write byte-identical; PNG tEXt and iTXt write byte-identical; a PDF Info edit matches (xref rebuilt; compare the parsed result and the xref entries); PS DSC rewrite matches.

**P13d Maker writers** (crate `pg-et-write-makers`). Modules: WriteCanonRaw.pl (644; CRW and CR2 write, MakerNotes builder if not already in P11a), WriteExif helpers for makers (RebuildMakerNotes write), Canon WriteCanon, CanonCustom write, CanonVRD write (WrapDR4, ProcessCanonVRD OutFile), Nikon ProcessNikonEncrypted write, Sony WriteEnciphered and FinishARW and WriteSR2 and WriteMRW use, PhaseOne WritePhaseOne, Kodak WriteKodakIFD, Samsung WriteSTMN, FujiFilm WriteRAF, PanasonicRaw WriteJpgFromRaw/WriteDistortionInfo/PatchRawDataOffset, Sanyo FixOffsets, Olympus/Pentax table writes (through WriteExif/WriteBinaryData). est 6,000. Prereq: P13a, P11a-P11f read sides, P10 (raw read side, for PanasonicRaw and MRW). Done: per maker, one oracle-compared write (a Canon custom-function change, a Nikon serial-key re-encrypt, a Sony enciphered field, a Samsung STMN update, a RAF pointer fixup) matches byte-exact.

**P14 CLI, i18n, validation, dev tools.** Modules: Lang loader plus 18 locale tables (62k lines, generated), TagInfoXML.pm (852; XML dump and BuildLangModules), TagLookup.pm (14,155; generated lookup block plus AddFields/AddTags/TagExists/FindTagInfo), BuildTagLookup.pm (2,875; dev generator, port last, as an `xtask`), Shift.pl (643, write-side and option shifts; its read calls are in P2; the `eval "Shift$func"` becomes a closed enum), Validate.pm (701, less its read side in P2; the Validate option, its eval-string rules become per-rule predicates), Import.pm (377, less ReadJSON, which is in P2; CSV and JSON tag input for -csv= and -json=), HtmlDump.pm (937, less its read side in P2; -htmlDump output), the public API surface (ImageInfo, GetInfo, GetTagList, GetAllTags, GetWritableTags, GetAllGroups, GetFileType, CanWrite, CanCreate, AddUserDefinedTags, OrderedKeys), and the CLI front end, which is P14b. est 4,000. Prereq: P2-P13. Done: `-listx`-equivalent tag list matches; `-lang de` output matches on 20 tags; `-csv=` and `-json=` import and `-htmlDump` pages match; the CLI's default and `-j`/`-G`/`-n`/`-s`/`-csv` outputs match on the corpus.

**P14b CLI front end (O7).** Modules: the `exiftool` program script (argument parsing, `-stay_open`, argument files, output formatters, exit codes). Not inventoried yet. est 2,500 as a provisional placeholder, counted in the totals below until O7 is inventoried. Prereq: O7 inventory, P14. Done: CLI argument and exit-code cases from the inventory match the oracle.

Totals: the phase estimates sum to about 205.5k (P0 3.0k; read phases 174.0k; writers P13a-d 22.0k; P14 4.0k; P14b 2.5k provisional), plus generated data. The group figures (~170k) match the read phases (174.0k) within 4k; the 4k is the Lytro, Other.pm and Unknown.pm rows and the added helpers. Either way the estimate is ±40%; re-estimate after P2 (when the engine's real size is known).

### 7.1 Write prerequisites per writer family

| Writer family | Crate | Phase | Read phases it calls |
|---|---|---|---|
| Writer core, WriteJPEG, WriteTIFF/WriteExif, Set* packers, trailer write | `pg-et-write-core` | P13a | P1, P2, P3, P4 |
| XMP, IPTC, Photoshop, ICC_Profile write, XMPStruct write side | `pg-et-write-meta` | P13b | P5a, P5b |
| QuickTime write (WriteQuickTime) | `pg-et-write-containers` | P13c | P8a |
| RIFF/WebP write | `pg-et-write-containers` | P13c | P7a, P5a (XMP chunk) |
| PNG write | `pg-et-write-containers` | P13c | P6a, P5a (XMP chunk) |
| PDF and PostScript write | `pg-et-write-containers` | P13c | P9a, P5a |
| FLIF, JP2/JXL box writers, GIF write, MIE write | `pg-et-write-containers` | P13c | P5b, P6a, P6b |
| CRW/CR2, Canon, CanonCustom, CanonVRD, Nikon, Sony, PhaseOne, Kodak, Samsung, FujiFilm RAF, PanasonicRaw, MRW, Sanyo, Olympus/Pentax table writes | `pg-et-write-makers` | P13d | P10, P11a-P11e |

Computed D4 ordering (the choice itself stays in D4): the app's formats are JPEG and TIFF (they need P1-P4), XMP and IPTC (they need P5a-P5b), PNG (it needs P6a) and WebP (it needs P7a). The first write set that needs only P1-P5b is JPEG, TIFF, XMP and IPTC. PNG and WebP wait for P6a and P7a.

## 8. Verification

### 8.1 Oracle

`C:/Program Files/ExifTool/exiftool.exe` (13.60 with its `exiftool_files/lib`). The harness runs it through `pg-exiftool` in `-stay_open` mode (`crates/exiftool/src/lib.rs`). Commands: `-j -G1 -n` (numeric), `-j -G1` (print conversions), `-listx` (tag metadata, cross-check table generation), and for writes, the same argument list on a copy of the input, then `cmp` of the outputs.

Determinism. ExifTool's output order is by sort and FILE_ORDER, but Perl hash order is randomised (inventory). Run the oracle with `PERL_HASH_SEED=0` and `PERL_PERTURB_KEYS=0` where the bundled perl honours them (O3), and check that two oracle runs are identical before trusting any diff. That two-run check is a P0 exit item, and D6 waits for its result.

### 8.2 Corpora

1. **Synthetic fixtures** (`pg-et-testkit`): built from the table data. For each `TagDef`, a minimal file of the right container with the tag, format, count and a value set (boundary values, zero, negative, NUL-terminated and non-UTF-8 strings, NaN-like rational 0/0). This is the per-expression oracle: thousands of cases, cheap to run, and it covers conditions because the fixture also sets the `Make`/`Model`/`FirmwareVersion` values each Condition needs (the generator reads the Condition text to pick values; fixtures for Conditions that cannot be satisfied are reported).
2. **exiftool-written fixtures**: start from real files and use `exiftool.exe` to write fields (`-tag=value`), producing the write-side corpus for read tests and the input for write tests.
3. **Real files on this machine**: photos, PDFs, Office files, MP4 and MOV, audio, archives, RAW files that the user already has. Treat as private: the harness reads them locally, never uploads, and the mismatch report shows paths only in the local report. Pick a stratified sample per family (the inventory lists the families).
4. **Repo fixtures**: `crates/testkit` and `crates/jobs-testkit` may already hold images (check; O5). Use them where they cover a format.
5. **ExifTool's own test images are not on this machine** (per the task). The synthetic and real-file corpora replace them. Where the inventory says a quirk is "verify with the oracle on an X file", the phase owner must find or build that X file; list the ones that cannot be built on this machine in the phase report (for example Mac-only and some camera-specific ones).
6. **Corpus inventory (P0 exit).** Match each phase's done list against the files on this machine (O5). Mark each named sample as available, synthetic-only (built by `pg-et-testkit`), blocked (cannot be built on this machine), or needs-user (the user supplies the file). Names that need a mark: DICOM, Garmin FIT, Palm PRC/MOBI, pcapng, `.torrent`, ELF (P9b); HEIC iloc/iinf (P8a); GoPro GPMF MP4 and dashcam freeGPS (P8b); DNG, MRW, X3F, RWZ, R3D, CZI, MRC (P10); Google HDRP and Parrot mett (P11f). P0 does not close until each has a mark.

### 8.3 Comparison and normalisation

- Field-by-field over the JSON from both tools, keyed by `Group:Tag` and the `Name (N)` index. Compare the `Entry` list, not a pretty string.
- Volatile fields excluded: `ExifToolVersion`, `FileAccessDate`, `FileInodeChangeDate`, `FileCreateDate` (where the copy time changes it), `SystemTime`, `ProcessingTime`, `Directory` path separators (normalised), `NewGUID`, `NewUUID`, `Now`. Fixture mtimes are pinned.
- Time zone pinned for the oracle and the port (`TimeZone` option, inventory ExifTool.pm 2669; and the Windows zone for the oracle run). The port takes the zone from `Et` (section 2.4) and never writes the process environment; the harness sets `TZ` on the oracle child process only. Otherwise `ConvertUnixTime`, `GetUnixTime`, Composite GPSDateTime and Parrot/QuickTime time tags differ by machine time zone (inventory risk).
- Floats compared after `%.15g` formatting on both sides, and exact for integers.
- Binary outputs (thumbnails, `PreviewImage`, `ImageDataHash`): compare the hash the oracle prints. The port must produce the same bytes.

### 8.4 Mismatch reporting

Each mismatch is one JSON line: `{ phase, file (relative to corpus root), group, tag, kind (missing|extra|value|order|warning|error), expected, actual, module, table, key, expr_id, fixture_id }`. The reporter aggregates by `expr_id` and by `module` and prints the top N by count. Triage rule: a mismatch with an `expr_id` goes to that expression's owner; a mismatch with no expression goes to the table or module owner. Each unexplained mismatch blocks the phase gate. Each accepted difference (a deliberate quirk fix, or an oracle non-determinism) is recorded in the quirk register (section 9, D8) with the input that shows it.

### 8.5 Writes

For each write fixture: the output file from the port is compared to the output from `exiftool.exe` for the same command. Byte-equal is the default. Byte-equal is the gate for compressed output once the zlib and other builds are matched (D5, O4). Until then, compare decoded payloads and the offsets of every chunk or box, and record the gap.

## 9. Decisions

Status as of 2026-11-12. "Chosen" means the most proper option, applied in this document. "Open" needs your decision.

- **D1 Expression strategy. Chosen: an interpreter for the Perl subset (section 4.2).** This replaces the earlier hybrid recommendation, which was driven by avoiding work rather than by correctness.
- **D2 Dev-only tools (BuildTagLookup, TagInfoXML, HtmlDump, the TagNames.pod regeneration). Chosen: ported last, as an xtask in P14.** In scope.
- **D3 `.ExifTool_config` and `configFile`/piped `-` input. Chosen: run through the interpreter (section 4.5).** Residual gap: a construct outside the census is a load error, not an ignored line.
- **D4 Write order. Chosen: (ii), the app's own write formats first.** That is P13a with the subset of P13b to P13d that those formats need, then the rest. Which formats `pg-writeback` writes is O6, read before P13 is scheduled.
- **D5 Compressed-output parity. Chosen: byte parity.** The zlib, brotli, bzip2 and LZMA builds of `exiftool.exe` are identified (O4) and matched, so output bytes agree. Decoded payloads are a second check. Until O4 is answered, the byte gate is not in force, and the gap is recorded.
- **D6 Hash iteration order. Chosen: fixed order by FILE_ORDER and PRIORITY in the port, with the oracle pinned** (`PERL_HASH_SEED=0`, `PERL_PERTURB_KEYS=0`) once O3 confirms that the pin works.
- **D7 Platform-only paths and user-supplied Perl. Chosen: Mac code behind `cfg(target_os = "macos")`, marked untested on this machine. Filter runs through the interpreter (section 4.2). NoWarning runs through PCRE2 (section 4.4).** Neither is a documented deviation.
- **D8 Quirk register. Chosen: reproduce every defect, do not fix.** This matches plan decision 1, "faithful everywhere, ExifTool's data-losing quirks included". Each entry is checked against `exiftool.exe` and recorded as reproduced. A defect the oracle does not show is a question for the phase owner, not a fix. The earlier recommendation to fix defects is withdrawn. Output that is random by design (the AES IV, NewGUID and NewUUID) cannot be byte-matched; the comparison excludes those fields (section 8.3), and the AES IV entry stays as recorded in P9a.
- **D9 Regex dialect. Chosen: PCRE2 for every regex site (section 4.4).** This replaces the earlier mixture of hand scanners and `fancy-regex`.
- **D10 Plot colours and `rand`. Chosen: reproduce Perl's drand48 sequence** (`srand(141)` in Plot.pm), after the bundled perl's `rand` is checked (O1). Same sequence for NewGUID and NewUUID in tests. Done in P12.
- **D11 Large data placement. Chosen: `include_bytes!` for `Geolocation.dat`, generated Rust for the rest, and `Lang` in the default build.** A cargo feature for Lang would drop it from the default build, and that is a removal.
- **D12 Crate granularity. Chosen: the split in section 1.2.** Updated in this revision: `pg-et-perl` is added and `pg-et-exprfns` is removed.
- **D13 `pg-exiftool` during migration. Chosen: keep it until P13 passes, then remove it in one commit.**
- **D14 Licence and attribution. Open: your decision.** Recommendation: the derived crates under ExifTool's dual terms (Artistic 1.0 or GPL-1+), with the ExifTool notice. `BZZ.pm` is reimplemented clean-room from the DjVu format description, not from DjVuLibre's GPL source. Legal review before any public release. P6b waits on this.
- **D15 Perl for the P0 dumper. Chosen: `/usr/bin/perl`** (5.42.2, with `B::Deparse`; `Image::ExifTool` 13.60 loads), checked on 2026-11-12. P0 pins that path and checks its `rand` and hash behaviour against `exiftool.exe` (O1, O3).
- **D16 Warnings inside expressions. Chosen: reproduce.** Each expression evaluation returns its value along with the warnings it raised. Each done test lists the fixtures that show such warnings.

## 10. Open questions (inventory gaps)

- **O1 Perl availability for the dumper.** P0 needs a Perl that can `use Image::ExifTool` from `C:/Program Files/ExifTool/exiftool_files/lib` and run `B::Deparse`. Is a standalone perl on this machine? Does `exiftool.exe` (a packed perl) expose the library? If neither, P0 must fall back to parsing the `.pm` files, which is much weaker. Entry gate for P0 (section 7). Checked 2026-11-12: `/usr/bin/perl` is Perl 5.42.2 with `B::Deparse`, and `Image::ExifTool` 13.60 loads from the install directory. D15 chose it. The earlier note that `perl` is not on PATH was about PowerShell's PATH only. `exiftool.exe` is a packed binary; whether it exposes the library is not yet verified. Until one of these is answered, the D1 tier split and D10 stay open. The choice of perl is D15. Also check the bundled perl version for `rand`/drand48 (D10) and hash-seed env vars (O3).
- **O2 `File/RandomAccess.pm` is not inventoried.** The core engine, ParseArguments, ExtractInfo, Samsung trailer, FujiFilm RAF, MinoltaRaw and others depend on it (inventory references: core-A, cross-group lists). Its semantics (Seek with negative offsets, ReadLine with `$/`, NoBuffer, BinMode, FILE_PT/BUFF_PT, TESTED, ERROR) need an inventory before P1. This is a gate on P1 (section 7): P1 does not start until the inventory exists. Its semantics must match in pg-et-io: Seek with negative offsets, ReadLine with a caller-supplied `$/`, NoBuffer, BinMode, TESTED, ERROR.
- **O3 Oracle determinism.** Confirm that two oracle runs give identical JSON, and that `PERL_HASH_SEED=0` / `PERL_PERTURB_KEYS=0` affect `exiftool.exe` (inventory risk: hash order).
- **O4 zlib build of exiftool.exe.** Version and compression level. The gate for D5's byte parity, identified in P0.
- **O5 Test images on this machine.** `crates/testkit` and `crates/jobs-testkit` contents; any image corpus under the user's folders that can be used for the real-file corpus (and which can be used privately).
- **O6 Which formats `pg-writeback` writes.** Needed to choose D4 ordering. Read `crates/writeback/src` before P13.
- **O7 The `exiftool` command-line front end.** The inventory covers `lib/Image/ExifTool.pm` and the modules, not the front-end script that parses `-j`, `-G`, `-n`, `-csv`, `-tagsfromfile`, argument files, `-stay_open`, output formatting and exit codes. The output parity gate in P14 depends on it. Inventory it before P0 closes, as P14b (section 7), with its estimate.
- **O8 `Lytro.pm` is missing.** It is referenced (docs-archives coverage notes and cross-group list: "Import (ReadJSON) used by Lytro.pm") but has no module entry in any group. Assigned to P6a with est 180 in this revision (the file is 214 lines; it reads JSON through the P2 ReadJSON). Its inventory row must still be written before P6a starts.
- **O9 Count disagreement.** Expression counts differ between inventories for the same module (Writer.pl "3" versus "45 eval lines"; Garmin 700 is almost certainly a grep artefact; Canon "952" total with "about 900" text). P0's census replaces them; no decision should rely on the inventory numbers.
- **O10 Entry points not confirmed.** Several cross-group callers are "verify" in the inventory (GoPro `ProcessGP6` caller in QuickTime; Protobuf callers; `ProcessVRD` dispatch site (CanonVRD coverage notes say not located); the Olympus `ProcessDSS`/`ProcessORF` dispatch; GE `ProcessGE2` callers; Garmin/QuickTime FIT path). Confirm during P2 and P8a/P8b.
- **O11 Maker-note MOV and sub-table cross-references.** The QuickTime group references maker sub-tables (Canon CNTH/CNOP, Nikon NCTG, Sony rtmd, Samsung, Kodak, Pentax MOV, FujiFilm MOV, Minolta MOV1/2, Olympus MOV, GoPro GPMF). Phase gating (section 5.4) is designed for this, but the order of P8a versus the maker phases should be confirmed with the owners of P11.
- **O12 Shared mutable globals with other families.** `@grps` is written by Composite RawConvs in Olympus, SonyIDC, MPF and read by Exif.pm (4989, 5055): confirm that all readers are reached through the composite evaluator before it becomes a return value.
- **O13 Writer.pl load-time float self-test.** The `%writeValueProc` table is filtered at load by a float self-test (inventory Writer.pl risk). Decide whether the port always supports float writes (recommended) and whether the oracle machine passes the test.
- **O14 `Image::ExifTool::Extra` conflicts.** Several groups write `%Extra`-style pseudo-tags (Trailer, Composite, FileType). Confirm the table ownership so the registry does not get two owners.
- **O15 Non-ASCII and Windows path semantics.** The inventory's Windows path rules (wide names, long path, `EncodeFileName` with `CharsetFileName`) are described from source, not tested on this Windows machine with names that need them. Build a fixture set with such names before P2's gate. The exact names are not yet chosen. Before the gate, write them here: one name with a non-BMP character, one with U+00E9, one of 260 or more characters, and one with a space. Until then O15 is open. The symlink fixtures are skip-when-unprivileged and are not part of O15.
- **O16 The `.ExifTool_config` search path and `EXIFTOOL_HOME` behaviour** (10230-10312) on Windows: confirm where the oracle looks, so D3's declarative subset loads the same files.
- **O17 Composite dependency cycle detection.** The inventory says Perl only warns on Composite cycles (BuildCompositeTags, GetValue recursion risk). The port's worklist must produce the same warning and the same result on a cyclic fixture; confirm with a constructed cycle.
- **O18 Memory and recursion limits.** ExtractInfo recursion (resource forks, alternate files, PGF re-entry depth 10, Rawzor re-entry, Nikon/Canon sub-tables) and GetValue Composite recursion need explicit depth limits; the oracle's effective limits are not in the inventory.
- **O19 Time-zone source on Windows.** `localtime`, `strftime` and Time::Local depend on the Windows zone. Confirm that the port reads the same zone (Windows TZ API via `windows-sys` or the `chrono`/`time` crate with the local offset), and that DST behaviour matches on the oracle machine (inventory: TimeLocal's ActivePerl compensation).
- **O20 Estimates from line ranges.** The rows added or split in this revision (the P2 helpers and the Validate, HtmlDump and ReadJSON read sides, the P1 extras, the P8a/P8b split, Lytro, Other.pm, Unknown.pm and the P14b CLI placeholder) are estimated from line ranges or assumed, not taken from inventory rows. Re-inventory them before the phase estimate is used. The P8a/P8b split in particular is provisional.
