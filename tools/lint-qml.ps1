# The QML's lint (`just lint-qml`), failing on any warning: qmllint's, and the QML compiler's. Every binding and
# function must compile to C++ (qmlcachegen, at build time), none left to the interpreter. qmlcachegen falls back to
# bytecode quietly, and says why only with --verbose; qmllint's own compiler pass (--compiler) misses much of what it
# rejects, so qmlcachegen itself is run here as the build runs it. Run this after a build: it reads the types of the
# pg.gallery module (the Rust QObjects) from the last build's plugin.qmltypes.
#
# The module is laid out in target/qmllint/pg/gallery as the build lays it out (vendor/README.md, qt-build-utils): its
# qmldir, its qmltypes and, beside them, copies of the QML as the qmldir lists them (qml/Main.qml, ...), plus a
# resource file mapping the module's resource paths to them, which qmlcachegen resolves the module through. The
# copies are of the sources as they are now, so the QML can be linted again without a rebuild. qmllint matches a file
# to its qmldir entry (Ratings is a singleton only through it) by the path it was given, so the copies are linted.
# With absolute paths qmllint 6.12 prints no text, only its JSON report, so the warnings are printed from that. All
# are printed as crates/desktop/qml paths (the line numbers are the same).
$ErrorActionPreference = "Stop"
# The repo's .env (the Justfile loads it for recipes; a direct run loads it here): keys the environment does not set.
foreach ($line in Get-Content (Join-Path $PSScriptRoot "..\.env") -ErrorAction SilentlyContinue) {
    if ($line -match '^\s*([^#=\s][^=]*?)\s*=\s*(.*?)\s*$' -and -not [Environment]::GetEnvironmentVariable($Matches[1])) {
        [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2])
    }
}
# The Qt kit: QT_DIR (its bin and lib folders), from the environment or .env.
$Qt = $env:QT_DIR
if (-not $Qt) { throw "QT_DIR is not set: point it at the Qt kit folder (the one with bin and lib), in the environment or in the repo's .env." }
if (-not (Test-Path (Join-Path $Qt "bin\qmake.exe"))) { throw "QT_DIR ($Qt) has no bin\qmake.exe: point it at the Qt kit folder." }
$root = (Split-Path $PSScriptRoot -Parent).Replace('\', '/')

# The build script's output directory is build/pg-desktop-<hash>/ with stable cargo and build/pg-desktop/<hash>/ with the
# pinned nightly (cargo 1.101): both patterns, the newest file wins.
$types = Get-ChildItem -Path @(
        "$root/target/*/build/pg-desktop-*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes",
        "$root/target/*/build/pg-desktop/*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes") `
    -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $types) { throw "No pg.gallery module in target/: build the app first (just build)." }

$imports = "$root/target/qmllint"
$module = "$imports/pg/gallery"
if (Test-Path $imports) { Remove-Item -Recurse -Force $imports }
New-Item -ItemType Directory -Force "$module/qml" | Out-Null
Copy-Item $types.FullName, "$($types.DirectoryName)/qmldir" $module
Copy-Item "$root/crates/desktop/qml/*.qml" "$module/qml"
$names = (Get-ChildItem "$module/qml/*.qml").Name
$entries = @("<file alias=`"qmldir`">$module/qmldir</file>") + ($names | ForEach-Object { "<file alias=`"qml/$_`">$module/qml/$_</file>" })
$qrc = "$module/module.qrc"
"<RCC><qresource prefix=`"/qt/qml/pg/gallery`">$($entries -join '')</qresource></RCC>" | Set-Content $qrc

$warnings = [System.Collections.Generic.List[string]]::new()
function Add-Warning([string]$path, [string]$line, [string]$column, [string]$message) {
    $name = $path.Replace('\', '/').Replace("$module/qml/", "crates/desktop/qml/")
    $warnings.Add("${name}:${line}:${column}: $message")
}

$report = & "$Qt/bin/qmllint.exe" -I $imports --json - ($names | ForEach-Object { "$module/qml/$_" }) |
    Out-String | ConvertFrom-Json
foreach ($file in $report.files) {
    foreach ($w in $file.warnings) { Add-Warning $file.filename $w.line $w.column "$($w.message) [$($w.id)]" }
}

# qmlcachegen says why it left something to the interpreter (--verbose), and lists what it compiled and what it
# didn't (--dump-aot-stats, into <output>.aotstats): the verdict, should a reason go unsaid.
$compiled = 0
$functions = 0
# Its findings come on stderr, which isn't an error here.
$ErrorActionPreference = "Continue"
foreach ($name in $names) {
    $out = "$imports/$name.cpp"
    $log = & "$Qt/bin/qmlcachegen.exe" -i "$module/qmldir" --resource $qrc --resource-path "/qt/qml/pg/gallery/qml/$name" `
        --verbose --dump-aot-stats --module-id pg.gallery -o $out "$module/qml/$name" 2>&1
    if ($LASTEXITCODE -ne 0) { throw "qmlcachegen failed on ${name}:`n$($log -join "`n")" }
    # Each finding is one "Warning:"/"Error:" line (the source line and a caret under it follow).
    foreach ($l in $log) {
        if ("$l" -match '^(?:Warning|Error): (.+?):(\d+):(\d+): (.*)$') {
            Add-Warning $Matches[1] $Matches[2] $Matches[3] $Matches[4]
        }
    }
    foreach ($entry in (Get-Content "$out.aotstats" -Raw | ConvertFrom-Json).modules.moduleFiles.entries) {
        $functions++
        if ($entry.codegenResult -eq 0) { $compiled++; continue }
        Add-Warning "$module/qml/$name" $entry.line $entry.column `
            "$($entry.functionName) is left to the interpreter: $($entry.message) [aot]"
    }
}

# Both tools report some findings alike.
$unique = @($warnings | Select-Object -Unique)
$unique
if ($unique.Count -gt 0) {
    "$($unique.Count) QML warning(s); $compiled of $functions bindings and functions compiled to C++."
    exit 1
}
"QML: no warnings; all $functions bindings and functions compiled to C++ ($($names.Count) files)."
