# Font fallback: Qt's first-miss cost on Windows (2026-11-26, plan)

The first time any text needs a glyph its font lacks, Qt on Windows loads every installed font family to build its
fallback list: once per process, about 370 ms on the GUI thread, under the font database's lock. Until commit
`48754c84` the startup window paid it before its first frame ("Views ▾": Segoe UI has no U+25BE). That commit gives our
own symbols and emoji fonts that have them (`Theme.symbolFamily`, `Theme.emojiFamily`), so the startup window no longer
misses a glyph. It can't do that for text we don't write: a folder or file name in another script, an emoji typed into
the name filter, the save-view dialog's line with a rating filter. This plan fixes the cost where it is, in Qt, and
carries the patch until Qt takes it.

- **Phase 1** makes the exhaustive part of the fallback list lazy: built only when the cheap curated part can't map a
  character. The font chosen for every character stays what it is today; only the cost moves.
- **Phase 2** asks DirectWrite's own fallback (`IDWriteFontFallback`) for a character's font, the way native Windows
  text does ([QTBUG-110935](https://bugreports-test.qt.io/browse/QTBUG-110935)). It changes which fonts are chosen,
  so it is reviewed on its own.
- Until upstream Qt has them, the app builds its own Qt: the static release build and a shared build for development
  and CI, from a `darktorres/qtbase` fork.

## Measurements (2026-11-26)

Release build (`relinc`), warm cache, `PG_TRACE=1`, Qt 6.12.0 (the prebuilt kit), 191 installed font families.

| Stage                     | Before `48754c84` | Note                                           |
|---------------------------|-------------------|------------------------------------------------|
| start → `init_done`       | ~35 ms            | the catalog's open is ~15 ms of it             |
| `qml_load` → `qml_loaded` | ~685 ms           | the window's QML is made; nothing is on screen |
| → `first_frame`           | ~850 ms           |                                                |

An instruction profile of `qml_load` → `qml_loaded` (Pin, `tools/profiling/callgraph`, 2.18 G instructions on the
main thread):

- 60%: `QWindowsDirectWriteFontDatabase::fallbacksForFamily` → `populateFamily` × 201, the fallback list, on the first
  unmapped glyph.
- 31%: `QWindowsDirectWriteFontDatabase::populateFontDatabase`, the families' names, on the first font use.
- 0.24%: `pg_*`, the bridges and the catalog reads.

- `qt.text.font.cmap` logged the trigger: `Mapping characters in "Views ▾"`, `Unmapped character "25be"`, then
  `Populate family` for each family from 0.437 s to 0.804 s.
- A one-line repro in `qml.exe` (no app): `Text { text: "Views ▾" }` populated 191 families; `"Views ▼"` (U+25BC,
  in Segoe UI) populated 1; `"▾"` with `font.family: "Segoe UI Symbol"` populated 2. `Text { text: "⭐" }` populated 191
  with the default font and 2 with `font.family: "Segoe UI Emoji"`.
- Every glyph the window needed was found early in the list: MS UI Gothic was fallback #3 (▾ ▸ ▴ ▭ ▶ ☆ ✓ ✕), Segoe UI
  Emoji #5 (⏸ ⚠), Segoe UI Symbol #6 (⟲ ⟳). All of them are in the curated part (below); the other 185 families were
  populated only to be ranked.

After `48754c84`, on a scratch copy of the full library, 264 strings of the startup window mapped with none missing,
offscreen. Windowed runs before its emoji part showed the first frame at about 440–580 ms, with the population moved
after the first frame. The final commit has no windowed timing yet (Verification, below).

## Where the cost is in Qt

All of it is in qtbase, `src/gui/text`, at 6.12.0:

- `QFontEngineMulti` (`qfontengine.cpp`) keeps engine 0 (the font asked for) and one engine per fallback family, made
  lazily. On the first glyph engine 0 lacks, `glyphIndex` and `stringToCMap` call `ensureFallbackFamiliesQueried`
  (`:2010`), which asks `qt_fallbacksForFamily` for the whole list at once (`:2016`).
- `fallbacksForFamily` (`qfontdatabase.cpp:706`) holds `fontDatabaseMutex()` throughout, caches the list per family,
  style, style hint and script, and asks the platform database.
- `QWindowsDirectWriteFontDatabase::fallbacksForFamily` (`windows/qwindowsdirectwritefontdatabase.cpp:383-392`) returns
  four parts in order: `familiesForScript` (Segoe UI Emoji for the emoji script), `familyForStyleHint` (Tahoma by
  default), `extraTryFontsForFamily` (the user language's try-fonts, then Segoe UI Emoji and Segoe UI Symbol) and
  `QPlatformFontDatabase::fallbacksForFamily`.
- `QPlatformFontDatabase::fallbacksForFamily` (`qfontdatabase.cpp:636-704`) calls `ensurePopulated()` on every family
  in the database (`:653-656`) to filter them by writing system and rank them by style. On DirectWrite a family's
  population opens its font faces and reads their localized names: about 2 ms each. That is the 370 ms.
- Two more callers build the same list: `QFontDatabasePrivate::loadEngine` builds it at once for a font with a family
  list (`request.fallBackFamilies`, `:841-853`), and `findFont`'s fallbacks pass (`:2767-2850`) builds it when the
  family asked for isn't installed, or when an emoji needs a colour font and the family isn't one (`:2719-2722`).

Qt has known this cost for years: [QTBUG-71737](https://bugreports-test.qt.io/browse/QTBUG-71737) (the first paint
waits for the fallback cache, Qt 5, closed), [QTBUG-74729](https://bugreports-test.qt.io/browse/QTBUG-74729) (window
creation blocks 200–500 ms on fallback families, closed as a duplicate) and
[QTBUG-85919](https://bugreports.qt.io/browse/QTBUG-85919) (the first line takes over half a second, open). No fix of
the eager population was found up to 6.12.

## What the app can't do itself

- **A pre-warm on a worker thread.** `fallbacksForFamily` holds the font database's lock for the whole 370 ms. A GUI
  thread that loads any font meanwhile waits on it, so a pre-warm before the first frame delays it, and one after it
  moves the stall rather than removing it.
- **A family list** (`font.families`). `loadEngine` builds the whole fallback list at once for one (`:841-853`).
- **The GDI font database** (`QT_QPA_PLATFORM=windows:fontengine=gdi`). It saved 100–180 ms here, renders text
  differently, and still builds the list.
- **Application fallback families** (`QFontDatabase::addApplicationFallbackFontFamily`). They are put in front of the
  platform's list (`qfontdatabase.cpp:720-721`), which is still built in full.
- **Finding emoji in QML.** Qt's JavaScript accepts `/\p{Extended_Pictographic}/u` but matches nothing, so QML can't
  split a string it didn't build into emoji and words.

What the app did (`48754c84`) is right whatever Qt does: Segoe UI Symbol is drawn to match Segoe UI, where MS UI Gothic
was a poor pick. It does not reach the text in the first paragraph.

## Phase 0: prototype and measure

Before any build or upstream work, check that Phase 1 removes the cost.

1. Check out qtbase at `v6.12.0` in a short path (`D:\src\qtbase`), and write Phase 1 there.
2. Build qtbase alone, shared, release with debug info, configured as the prebuilt kit was (its `mkspecs/qconfig.pri`
   and `lib/cmake/Qt6/QtBuildInternalsExtra.cmake` list the features).
3. Copy the prebuilt kit (`C:/Qt/6.12.0/msvc2022_64`) to a scratch folder, replace its `Qt6Gui.dll` (the font code is
   in QtGui) with the built one, and point `PG_QT_DIR` at the copy. A drop-in is valid only for a build of the same
   version and features; it is for measuring, not for shipping.
4. Measure against a scratch catalog (`PG_HOME`, `PG_CATALOG_DIR`, `PG_PREFS`), windowed: offscreen uses Qt's FreeType
   font database, not DirectWrite, and can't show this cost.
   - The `qml.exe` repros above (▾ and ⭐ with the default font): the families populated, and the time.
   - The app at `48754c84`'s parent (`02eb7133`), which still misses "Views ▾" before its first frame: `PG_TRACE=1`
     stages, five runs each, with and without the patched `Qt6Gui.dll`.
   - A string in a script no curated font has, so the tail is still built when it must be: the same families as today.

Exit: the misses cost single-digit milliseconds, and every font chosen is the one stock Qt chooses (the
`qt.text.font.cmap` log's `Fallback #n, family`) for a corpus of strings: our symbols and emoji, Latin, CJK, Arabic,
Devanagari, Thai.

## Phase 1: the exhaustive part, built only when needed

### Design

The fallback list becomes two parts: a **head**, which the platform can name cheaply, and a **tail**, the exhaustive
enumeration. The tail is built only when the head can't map a character.

- `QPlatformFontDatabase` gets a second query beside `fallbacksForFamily`, for the head. The name and shape are settled
  in review; a proposal:

  ```cpp
  // The first fallback families for `family`, cheaply: no family is populated beyond those it names. The rest of
  // fallbacksForFamily's list is built only if these can't map a character. The default returns an empty list,
  // which leaves a platform as it is today.
  virtual QStringList preferredFallbacksForFamily(const QString &family, QFont::Style style,
                                                  QFont::StyleHint styleHint,
                                                  QFontDatabasePrivate::ExtendedScript script) const;
  ```

  `QWindowsDirectWriteFontDatabase` returns its first three parts there, and the GDI `QWindowsFontDatabase` its first
  four (`qwindowsfontdatabase.cpp:1170-1180`, with the EUDC fonts); each one's `fallbacksForFamily` stays the full list. The head plus the tail, with
  the head's families removed from the tail, is the list in today's order, so the first family that maps a character
  is the same family as today.
- `QFontEngineMulti` asks for the head first. `ensureFallbackFamiliesQueried` sets the head; a new
  `ensureTailQueried` appends the tail when a character is unmapped after every head engine has been tried. Engines are
  only appended, so a glyph's high byte (its engine's index, `highByte`) stays valid, and the 256-engine limit applies
  as now. A platform with no head (the default) queries the full list once, as today.
- `fallbacksForFamily` (static, `qfontdatabase.cpp:706`) caches the head and the full list under separate keys.
- `findFont`'s fallbacks pass tries the head's families first, and builds the full list only if none matches. For an
  emoji with a non-colour family, that is `familiesForScript`'s Segoe UI Emoji, the first family it tries today.
- `loadEngine` with a family list (`:841-853`) passes the head, not the full list, to `setFallbackFamiliesList`, with
  the tail to follow on a miss.
- `QFont::ContextFontMerging` (opt in) tries every fallback on the whole string: it builds the full list, as today.

### Behaviour

The same font for every character as stock Qt; Phase 0's corpus checks it. What changes is when the tail is built:
never, for text the head maps, which is every string the window shows today.

### Tests (qtbase)

- `tests/auto/gui/text/qfontdatabase`: a character the head maps does not populate the other families
  (`QFontDatabasePrivate`'s family list, `populated` per family), and one it doesn't builds the tail once.
- `tests/auto/gui/text/qfont` or `qtextlayout`: the font chosen for each character of a mixed corpus is the same with
  and without the head (a test-only switch to the old order).
- A benchmark in `tests/benchmarks/gui/text`: the first layout of a string with one unmapped symbol.

## Phase 2: DirectWrite's per-character fallback (QTBUG-110935)

Phase 1 keeps Qt's choices. Those choices are Qt's own lists, not Windows': the try-fonts are by the user's language
(`extraTryFontsForFamily`), and the tail is alphabetical by family. QTBUG-110935 reports wrong glyphs from that order
once custom fonts are installed, and our log shows ▾ drawn in MS UI Gothic, a Japanese UI font, where Windows draws it
in Segoe UI Symbol.

### The per-character query

- A per-character query on `QPlatformFontDatabase`, default empty:

  ```cpp
  // The family the platform draws `ucs4` in when `fontDef`'s family lacks it, or an empty string.
  virtual QString fallbackFamilyForCharacter(char32_t ucs4, const QFontDef &fontDef,
                                             QFontDatabasePrivate::ExtendedScript script) const;
  ```

- `QWindowsDirectWriteFontDatabase` answers it with `IDWriteFontFallback::MapCharacters`: the system fallback
  (`IDWriteFactory2::GetSystemFontFallback`, Windows 8.1 and later) with an `IDWriteTextAnalysisSource` over the
  character and the user's locale, the system font collection, the family asked for, and the font's weight, style and
  stretch. The mapped font's family name (`IDWriteFont::GetFontFamily`, `GetFamilyNames`, en-us) is the answer. No
  family is populated to find it. The factory is the database's shared one; DirectWrite's shared factory objects may be
  used from any thread, which a QFontEngineMulti off the GUI thread needs (verify for `IDWriteFontFallback`).
- `QFontEngineMulti`, on a glyph engine 0 lacks, asks it first. A family not yet in its list is appended (a new engine
  index), and the glyph is taken from it if it has one. Only when the answer is empty, or has no glyph, does it go on to
  the head and then the tail (Phase 1).
- The answers are cached per multi engine (the family list itself) and per database for (family, weight, style,
  character), under the database's lock.
- `findFont`'s emoji pass asks for a colour font for a representative emoji instead of walking the list.

### What Phase 2 changes

Fonts are chosen as Windows chooses them: ▾ from Segoe UI Symbol, CJK by the system's per-locale fonts. That is a
visible change, and it is reviewed as one. Phase 0's corpus is rerun and each difference listed.

### Phase 2's tests

The corpus of Phase 1, now against `MapCharacters`' answers, and a font installed at run time
(`QFontDatabase::addApplicationFont`) that must not take the place of the system's choice (QTBUG-110935's case).

## Upstream

- File a QTBUG for Phase 1, the eager population on the first miss, with this page's numbers and the `qml.exe`
  repro; link QTBUG-85919, QTBUG-74729 and QTBUG-71737. Comment on QTBUG-110935 for Phase 2.
- Changes go through Qt's Gerrit (codereview.qt-project.org) against `dev`, one change per phase, Phase 1 first, with
  a cherry-pick request to the 6.x branch we use. Contributing needs a Qt Account and the Qt Contribution Agreement
  signed by its author.
- The patch for this repo is the change as it is under review, rebased when review changes it.

## Carrying the patch

Until Qt has the change in a release we use, the app runs on our build of Qt.

- **The fork.** `github.com/darktorres/qtbase`, a branch at the `v6.12.0` tag with the phases' commits, marked
  `photo-gallery` as `vendor/README.md` does for the crates. `vendor/README.md` gets a Qt section: the changes, the
  upstream changes they track, and when to drop them.
- **The release build.** `tools/build_static_qt.py` unpacks Qt's single-archive zip and replaces `qtbase` with the
  fork's tree at a pinned commit (an archive, checked by SHA-256), or applies the fork's commits as a patch file kept
  under `tools/qt/`. The manifest records the commit.
- **Development.** The prebuilt kit can't be patched: the font code is in `Qt6Gui.dll`, and a drop-in is valid only
  for the same version and features. A shared build from the same source, the modules the app uses
  (`build_static_qt.py`'s `SUBMODULES`), becomes the kit `PG_QT_DIR` names. `build_static_qt.py` gains a `--shared` mode
  (or a sibling script); the work folder's layout is the same.
- **CI.** `install-qt-action` installs the stock kit. The Windows job gets the shared build instead: built once per pin,
  published as a release asset of the fork (a zip, its SHA-256 pinned beside the Qt pins in the Justfile), and
  downloaded and checked as `tools/setup_ffmpeg.py` does FFmpeg. The Linux job keeps the stock kit: Phase 1's code is
  common, but the platform without a head (fontconfig) behaves as today, and Phase 2 is Windows only.
- **Updates.** `tools/fork-updates.py` compares crate forks with crates.io; it gains Qt: the fork's base tag against
  Qt's newest release of the pinned minor version, and whether upstream has merged the changes.
- **Dropping it.** When a Qt release has both phases, the pins go back to Qt's archive and the stock kit, and the fork
  is archived.

## The app, after Phase 1

- The fonts of `48754c84` stay: they are the right fonts for those glyphs, whatever Qt costs.
- The splits made only to keep emoji out of a mixed text (the HUD's info line, the toast's mark) may stay or go back
  to one text: with Phase 1 a mixed text's emoji costs one head lookup. Decide after Phase 0's numbers.
- The save-view dialog's line needs no change.
- `populateFontDatabase` (31% of the load) is next: the families' names, read on the first font use. It is a separate
  question (whether DirectWrite's names can be read lazily, or overlap the QML load), not part of this plan.

## Verification

- Every measurement runs against a scratch catalog (`PG_HOME`, `PG_CATALOG_DIR`, `PG_PREFS`), never the data folder.
- Font behaviour and timing are measured windowed (DirectWrite); offscreen runs (FreeType) check only that each string
  maps. A windowed run is agreed first, as it puts a window on the desktop.
- The hour-long static Qt build is not run to check a recipe step that an earlier stage can check (`--configure-only`,
  the qtbase-only build of Phase 0).
- The app's checks on the patched kit: `just check`, `just test`, `just qmltest`, `just lint-qml`, and the screenshot
  matrix (`just shots`), compared with the stock kit's: any difference in a picture is a font choice and must be one
  Phase 2 lists.

## Decisions

1. Upstream at all, and under whose account (the Contribution Agreement is the author's).
2. Phase 2 now, or after Phase 1 is merged upstream.
3. Development on our shared Qt (the same behaviour as the release) or on the stock kit (no build, the 370 ms in
   development only). This plan assumes our build.
4. The patch carried as a fork tree or as a patch file over Qt's archive.

## Sources

- Qt 6.12.0 source, `C:/Qt/6.12.0/Src/qtbase`: `src/gui/text/qfontengine.cpp`, `qfontdatabase.cpp`,
  `qplatformfontdatabase.h`, `windows/qwindowsdirectwritefontdatabase.cpp`, `windows/qwindowsfontdatabasebase.cpp`,
  `src/plugins/platforms/windows/qwindowsintegration.cpp`, `src/plugins/platforms/offscreen/qoffscreenintegration.cpp`.
- [QTBUG-110935](https://bugreports-test.qt.io/browse/QTBUG-110935): use `IDWriteFontFallback` on Windows (open).
- [QTBUG-85919](https://bugreports.qt.io/browse/QTBUG-85919): the first rendered line takes over half a second (open).
- [QTBUG-74729](https://bugreports-test.qt.io/browse/QTBUG-74729): window creation blocked by fallback families.
- [QTBUG-71737](https://bugreports-test.qt.io/browse/QTBUG-71737): the first paint waits for the fallback cache.
- [QTBUG-129849](https://bugreports.qt.io/browse/QTBUG-129849): a DirectWrite font database regression in 6.8.0.
