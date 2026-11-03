# Instruction profile of the gallery, callgrind's format, one file per thread: runs target/release/gallery.exe
# under Pin with callgraph.dll (build.sh) until a startup stage (crates/trace/src/lib.rs), then names the functions
# Pin could only name by address (symbolize.py). Read the result with tools/profiling/cgtab.py or KCachegrind.
#
#   profile.ps1 -Out D:\profiles\ready [-Stop pg_stage_ready] [-Start STAGE] [-- gallery arguments]
#
# The app runs with this shell's environment: set PG_CATALOG_DIR and PG_PREFS for a scratch catalog first. Under Pin
# the start takes some 50 times longer; the window shows meanwhile. -Stop '' runs until the app exits.
param(
    [Parameter(Mandatory)][string]$Out,
    [string]$Stop = 'pg_stage_ready',
    [string]$Start = '',
    [string]$Exe = "$PSScriptRoot\..\..\..\target\release\gallery.exe",
    [string]$Qt = 'C:/Qt/6.12.0/msvc2022_64',
    [Parameter(ValueFromRemainingArguments)][string[]]$AppArgs = @()
)
$ErrorActionPreference = 'Stop'
$pin = "$env:LOCALAPPDATA\Programs\sde\pinkit\pin.exe"
$tool = "$PSScriptRoot\..\..\..\target\pin\callgraph.dll"
if (-not (Test-Path $tool)) { throw "no ${tool}: run tools/profiling/callgraph/build.sh first" }
New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null

$knobs = @('-o', $Out)
if ($Stop) { $knobs += '-stop', $Stop }
if ($Start) { $knobs += '-start', $Start }
$sw = [Diagnostics.Stopwatch]::StartNew()
& $pin -t $tool @knobs -- (Resolve-Path $Exe).Path @AppArgs
Write-Output ("pin: exit {0} after {1:N0} s" -f $LASTEXITCODE, $sw.Elapsed.TotalSeconds)

$files = Get-ChildItem (Split-Path $Out) -Filter "$(Split-Path $Out -Leaf).*-*" | Sort-Object LastWriteTime |
    Group-Object { $_.Name -replace '-\d+$', '' } | Select-Object -Last 1 -ExpandProperty Group
python "$PSScriptRoot\symbolize.py" --path "$Qt/bin" @($files.FullName)
