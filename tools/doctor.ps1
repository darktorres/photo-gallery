# Checks this machine for what the build and the tests need, and says how to fix each missing part (`just doctor`).
# Reads .env through `just`: run it that way, so the values in .env count. It builds nothing and changes nothing.
#
# Required parts: cargo, the Qt kit, the FFmpeg headers and DLLs, and libclang. Optional parts are used by some tests
# and by the WSL recipes; a missing optional part only says so. Exit code 1 when a required part is missing.

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$script:missingRequired = $false

function Show([string]$Status, [string]$Name, [string]$Detail) {
    Write-Host ('{0,-9} {1,-12} {2}' -f $Status, $Name, $Detail)
}

function Require([bool]$ok, [string]$name, [string]$okDetail, [string]$fix) {
    if ($ok) {
        Show 'ok' $name $okDetail
    } else {
        Show 'MISSING' $name $fix
        $script:missingRequired = $true
    }
}

function Optional([bool]$ok, [string]$name, [string]$okDetail, [string]$fix) {
    if ($ok) {
        Show 'ok' $name $okDetail
    } else {
        Show 'optional' $name $fix
    }
}

if (Test-Path -LiteralPath (Join-Path $repo '.env')) {
    Show 'ok' '.env' 'read by just'
} else {
    Show 'note' '.env' 'none: copy .env.example to .env to set this machine''s paths'
}

# cargo
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
Require ($null -ne $cargo) 'cargo' $cargo.Source 'install Rust with rustup (https://rustup.rs); the toolchain comes from rust-toolchain.toml'

# Qt: the kit's qmake is what cxx-qt-build runs.
$qt = $env:QT_DIR
$qmake = if ($qt) { Join-Path $qt 'bin/qmake.exe' } else { '' }
Require ($qmake -and (Test-Path -LiteralPath $qmake -PathType Leaf)) 'Qt (QT_DIR)' $qt `
    'set QT_DIR in .env to the Qt 6.12.0 msvc2022_64 kit (the folder with bin\qmake.exe), installed with the Qt Online Installer'

# FFmpeg: the headers are what build.rs generates from, and the DLLs are what the app and the tests load.
$ffmpeg = if ($env:FFMPEG_DIR) { $env:FFMPEG_DIR } else { Join-Path $repo '.deps/ffmpeg' }
$ffmpegOk = (Test-Path -LiteralPath (Join-Path $ffmpeg 'include/libavcodec/avcodec.h') -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $ffmpeg 'bin') -PathType Container)
Require $ffmpegOk 'FFmpeg' $ffmpeg 'run `just setup` to fetch the pinned build, or set FFMPEG_DIR in .env to an FFmpeg 9.0 shared build'

# libclang: bindgen (pg-video's build script) loads it. LIBCLANG_PATH names its folder; LLVM_DIR is the LLVM install.
$clangDirs = @(
    $env:LIBCLANG_PATH
    if ($env:LLVM_DIR) { Join-Path $env:LLVM_DIR 'bin' }
    Join-Path $env:ProgramFiles 'LLVM/bin'
) + ($env:PATH -split ';')
$clang = $clangDirs | Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $_ 'libclang.dll') -PathType Leaf) } |
    Select-Object -First 1
Require ($null -ne $clang) 'libclang' $clang `
    'install LLVM (winget install LLVM.LLVM), then set LIBCLANG_PATH in .env to its bin folder (the one with libclang.dll)'

# Optional: the tests that need ExifTool and ffmpeg (they skip without them, unless CI's PG_REQUIRE_TOOLS makes them fail).
$exiftool = if ($env:PG_EXIFTOOL) { $env:PG_EXIFTOOL } else { 'exiftool' }
$exiftoolCmd = Get-Command $exiftool -ErrorAction SilentlyContinue
Optional ($null -ne $exiftoolCmd) 'ExifTool' $exiftoolCmd.Source `
    'the metadata tests skip without it: install ExifTool, or set PG_EXIFTOOL in .env to its exe'

$ffmpegExe = if ($env:PG_FFMPEG) { $env:PG_FFMPEG } else { Join-Path $ffmpeg 'bin/ffmpeg.exe' }
$ffmpegCmd = Get-Command $ffmpegExe -ErrorAction SilentlyContinue
Optional ($null -ne $ffmpegCmd) 'ffmpeg exe' $ffmpegCmd.Source `
    'the video tests skip without it: set PG_FFMPEG in .env to ffmpeg.exe, or run `just setup`'

# Optional: the WSL distro the perf, fuzz and wsl-check recipes run in (PG_WSL_DISTRO, default Ubuntu-26.04).
$distro = if ($env:PG_WSL_DISTRO) { $env:PG_WSL_DISTRO } else { 'Ubuntu-26.04' }
$wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
# `wsl --list` writes UTF-16, so each line comes with NUL characters between its letters: strip them.
$distros = if ($wsl) { @(wsl.exe --list --quiet 2>$null | ForEach-Object { ($_ -replace "[`0\r]", '').Trim() }) } else { @() }
$hasDistro = $distros -contains $distro
Optional $hasDistro "WSL $distro" 'present' `
    "the perf, fuzz-smoke and wsl-check recipes need it: install WSL and the distro (wsl --install -d $distro), or set PG_WSL_DISTRO in .env"

if ($script:missingRequired) {
    Write-Host ''
    Write-Host 'Missing required parts: fix them above, then run `just doctor` again.'
    exit 1
}
Write-Host ''
Write-Host 'Every required part is in place.'
