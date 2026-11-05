# The QML tests (S-14.3, `just qmltest`): Qt Quick Test, offscreen, on the QML that stays in QML.
#
# The components are staged as the pg.gallery module (target/qmltest/pg/gallery), the way the app has them: the stubs
# of the Rust types (crates/desktop/tests/qml/stubs), and every real QML file, each with a qmldir entry, so the
# singleton (Ratings) resolves as it does in the app. Without the module, a component beside a singleton's file (GridCell
# beside Ratings) takes the file's own type, and Ratings.symbol is not a function. The tests import pg.gallery and check
# the components only; the stubs stand in for GalleryModel, Shell and the rest.
param([string]$Qt = "C:/Qt/6.12.0/msvc2022_64")

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$staged = Join-Path $root "target\qmltest"
if (Test-Path $staged) { Remove-Item -Recurse -Force $staged }
$module = Join-Path $staged "pg\gallery"
New-Item -ItemType Directory -Force $module | Out-Null
Copy-Item (Join-Path $root "crates\desktop\tests\qml\stubs\pg\gallery\*") $module
$entries = [System.Collections.Generic.List[string]]::new()
$entries.AddRange([string[]](Get-Content (Join-Path $module "qmldir")))
foreach ($file in Get-ChildItem (Join-Path $root "crates\desktop\qml") -Filter *.qml) {
    Copy-Item $file.FullName $module
    if ($file.BaseName -eq "Ratings") {
        $entries.Add("singleton Ratings 1.0 Ratings.qml")
    } else {
        $entries.Add("$($file.BaseName) 1.0 $($file.Name)")
    }
}
Set-Content -Path (Join-Path $module "qmldir") -Value $entries

$env:PATH = "$Qt/bin;" + $env:PATH
$env:QT_QPA_PLATFORM = "offscreen"
$out = Join-Path $root "target\qmltest.txt"
& "$Qt/bin/qmltestrunner.exe" -input (Join-Path $root "crates\desktop\tests\qml") -import $staged -o "$out,txt"
$code = $LASTEXITCODE
Get-Content $out
exit $code
