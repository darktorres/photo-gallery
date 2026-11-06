# The QML tests (S-14.3, `just qmltest`): Qt Quick Test, offscreen, on the QML that stays in QML.
#
# The components are staged as the pg.gallery module (target/qmltest/pg/gallery), the way the app has them: the stubs
# of the Rust types (crates/desktop/tests/qml/stubs), and every real QML file, each with a qmldir entry, so the
# singleton (Ratings) resolves as it does in the app. Without the module, a component beside a singleton's file (GridCell
# beside Ratings) takes the file's own type, and Ratings.symbol is not a function. The tests import pg.gallery and check
# the components only; the stubs stand in for GalleryModel, Shell and the rest.
#
# The tests run in a host built from crates/desktop/tests/qml_host (Qt Quick Test's main, as qmltestrunner has it) with the
# app's image providers as stubs, so the image:// URLs the tests pass resolve as they do in the app.
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
# Offscreen draws text with FreeType, which reads the fonts from QT_QPA_FONTDIR or else from <Qt>/lib/fonts. Qt no longer
# ships fonts, so that folder is missing and every text draw warns. The app sets the same folder (crates/desktop/cpp/
# gui_app.cpp); this runner doesn't go through that code, so the script sets it.
if (-not $env:QT_QPA_FONTDIR) { $env:QT_QPA_FONTDIR = Join-Path $env:WINDIR "Fonts" }

# The host (CMake, the Qt kit's packages): built into target\qml_host, then run with qmltestrunner's arguments.
$host_build = Join-Path $root "target\qml_host"
cmake -S (Join-Path $root "crates\desktop\tests\qml_host") -B $host_build "-DCMAKE_PREFIX_PATH=$Qt"
if ($LASTEXITCODE -ne 0) { throw "configuring the QML test host failed" }
cmake --build $host_build --config Release
if ($LASTEXITCODE -ne 0) { throw "building the QML test host failed" }

$out = Join-Path $root "target\qmltest.txt"
& (Join-Path $host_build "Release\qml_host.exe") -input (Join-Path $root "crates\desktop\tests\qml") -import $staged -o "$out,txt"
$code = $LASTEXITCODE
Get-Content $out
exit $code
