# qmllint on crates/ui/qml, failing on any warning (`just lint-qml`). Run it after a build: it reads the types of
# the pg.gallery module (the Rust QObjects) from the last build's plugin.qmltypes.
#
# The module that cxx-qt-build writes can't be linted against as it is: its qmldir lists the QML as qml/Main.qml
# and so on beside itself, where there is no QML (Qt's CMake build copies it there; cxx-qt doesn't), and it doesn't
# name the module the models' base class comes from (QAbstractListModel, QtQml.Models). So the module is laid out
# again in target/qmllint/pg/gallery: the qmldir with flat paths and that dependency, the qmltypes, and copies of
# the QML. The files are linted there by absolute path, as part of their module: qmllint matches a file to its
# qmldir entry (Ratings is a singleton only through it) by the path it was given. With absolute paths qmllint
# 6.12 prints no text, only its JSON report, so the warnings are printed from that, as crates/ui/qml paths (the
# line numbers are the same).
param([string]$Qt = "C:/Qt/6.12.0/msvc2022_64")

$ErrorActionPreference = "Stop"
$root = (Split-Path $PSScriptRoot -Parent).Replace('\', '/')

$types = Get-ChildItem "$root/target/*/build/pg-ui-*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes" `
    -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $types) { throw "No pg.gallery module in target/: build the app first (just build)." }

$imports = "$root/target/qmllint"
$module = "$imports/pg/gallery"
if (Test-Path $imports) { Remove-Item -Recurse -Force $imports }
New-Item -ItemType Directory -Force $module | Out-Null
Copy-Item $types.FullName $module
$qmldir = (Get-Content "$($types.DirectoryName)/qmldir") -replace ' qml/', ' '
$qmldir + "depends QtQml.Models" | Set-Content "$module/qmldir"
Copy-Item "$root/crates/ui/qml/*.qml" $module

$files = (Get-ChildItem "$module/*.qml").Name | ForEach-Object { "$module/$_" }
$report = & "$Qt/bin/qmllint.exe" -I $imports --json - $files | Out-String | ConvertFrom-Json
$count = 0
foreach ($file in $report.files) {
    $name = $file.filename.Replace("$module/", "crates/ui/qml/")
    foreach ($w in $file.warnings) {
        "${name}:$($w.line):$($w.column): $($w.type): $($w.message) [$($w.id)]"
        $count++
    }
}
if ($count -gt 0) {
    "$count QML warning(s)."
    exit 1
}
"QML: no warnings ($($report.files.Count) files)."
