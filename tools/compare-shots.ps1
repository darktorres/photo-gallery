# Screenshot comparison (S-14.1, `just shots`): the working tree's gallery.exe against a reference recorded from a base
# commit, over the test-hook matrix, on one fixture, pixel by pixel. Exit code 1 when anything differs unexpectedly
# (or a run fails).
#
#   just shots -Record <commit>     # build <commit> once and record its screenshots as a reference
#   just shots                      # working tree against the reference (the only one, or -Base <commit>)
#   just shots -Expect apply-view   # these scenarios are declared to differ: reported, not failures
#   just shots -Self                # the working tree against itself: the matrix's own determinism
#   just shots -Only view0,apply-view
#
# How it works:
# - A reference is tools/shots/reference/<sha10>/: one PNG per scenario and manifest.json, which holds the full commit,
#   the fixture's hash, the Qt folder, the wait, and each scenario's crop hashes and settle line. It is recorded once,
#   from a scratch worktree at target\shots\base (built into target\shots\base-target). A comparison builds only the
#   working tree, so the base is not built again. Recording over an existing reference replaces it.
# - The fixture (tools/shots/fixture.py): a synthetic library (target\shots\library, made once; delete it to make
#   another) and, per side, a catalog built by that side's own binary (scan, import, thumbnails at 120 and 220
#   px) and saved as a template. A comparison refuses a reference whose fixture hash differs from the library's.
# - Each run starts from a copy of its side's template catalog, fresh preferences, and the library as it was
#   made: the turn and rating keys write into the original files, so Restore-Library puts them back in place
#   (same file, so the catalog's identity holds; the timestamp is put back too).
# - A difference is written as a diff image (the reference's pixels, with the differing ones red) under
#   target\shots\diff. Each run's log and output stay under target\shots\runs\<side>\<scenario>\.
#
# The matrix is the hooks of crates/desktop/qml/TestHooks.qml, each alone, plus a few together. A run's screenshot
# is taken when the window has settled (cpp/screenshot.cpp: ready, no image loading, no one-shot QML timer pending,
# no frame drawn for 300 ms). `-Wait` is the most to wait for that; a run that reaches it is reported UNSETTLED and
# not compared. Each run's settle log is <png>.settle.txt, with what the check saw.

[CmdletBinding()]
param(
    [string]$Record = "",
    [string]$Base = "",
    [switch]$Self,
    [double]$Wait = 10,
    [string]$Only = "",
    [string]$Expect = "",
    [string]$Python = "C:\Python312\python.exe"
)

$ErrorActionPreference = "Stop"
if ($Record -and $Self) { throw "-Record and -Self do not go together" }
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Shots = Join-Path $Repo "target\shots"
$Library = Join-Path $Shots "library"
$Pristine = Join-Path $Shots "library-pristine"
$VenvPython = Join-Path $Shots "venv\Scripts\python.exe"
$FixturePy = Join-Path $PSScriptRoot "shots\fixture.py"
$Requirements = Join-Path $PSScriptRoot "shots\requirements.txt"
$RefRoot = Join-Path $PSScriptRoot "shots\reference"

# The Qt and FFmpeg folders, from .cargo/config.toml (the same ones the build uses).
$config = Get-Content (Join-Path $Repo ".cargo\config.toml") -Raw
$qmake = [regex]::Match($config, 'QMAKE\s*=\s*"([^"]+)"').Groups[1].Value
$ffmpeg = [regex]::Match($config, 'FFMPEG_DIR\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $qmake -or -not $ffmpeg) { throw "QMAKE and FFMPEG_DIR are read from .cargo\config.toml; they are missing there" }
$env:PATH = (Split-Path $qmake) + ";" + (Join-Path $ffmpeg "bin") + ";" + $env:PATH
$env:QT_QPA_PLATFORM = "offscreen"

# The matrix: name -> the hook arguments, as one command line ({FIX}: the fixture, {RUN}: the run's folder). The
# JSON values carry their quotes escaped (\"), as the command line needs them.
$Scenarios = [ordered]@{
    "default"              = ""
    "size120"              = "--size=120"
    "select5"              = "--select=5"
    "view0"                = "--view=0"
    "viewer-zoom"          = "--view=0 --zoom=2"
    "viewer-loupe-borders" = "--view=0 --viewer-keys=LB"
    "viewer-zen"           = "--view=0 --viewer-keys=H"
    "viewer-one-to-one"    = "--view=0 --viewer-keys=1"
    "viewer-turn"          = "--view=0 --viewer-keys=>"
    "viewer-rate"          = "--view=0 --viewer-keys=r3"
    "filters"              = "--filters=1"
    "query"                = '--query={\"exts\":[\"png\"]}'
    "sort"                 = '--sort={\"key\":\"name\",\"descending\":true}'
    "menu-views"           = "--menu=views"
    "library"              = "--library=1"
    "apply-view"           = "--apply-view=alpha"
    "save-view"            = "--save-view=1"
    "borders"              = "--borders"
    "find-similar"         = "--find-similar={FIX}\notes.txt"
    "save-crops"           = "--save-crops={RUN}\crops"
    "filters-view0"        = "--filters=1 --view=0"
}
$onlyList = @($Only -split "," | Where-Object { $_ })
if ($onlyList.Count -gt 0) {
    foreach ($name in @($Scenarios.Keys)) { if ($name -notin $onlyList) { $Scenarios.Remove($name) } }
    if ($Scenarios.Count -eq 0) { throw "no scenario matches -Only ($Only)" }
}
$expectList = @($Expect -split "," | Where-Object { $_ })

function Build-Gallery([string]$Tree, [string]$TargetDir) {
    Push-Location $Tree
    try {
        $args = @("build", "-p", "pg-gallery")
        if ($TargetDir) { $args += @("--target-dir", $TargetDir) }
        & cargo @args | Out-Host
        if ($LASTEXITCODE) { throw "cargo build failed in $Tree" }
    } finally { Pop-Location }
    $dir = if ($TargetDir) { $TargetDir } else { Join-Path $Tree "target" }
    return (Join-Path $dir "debug\gallery.exe")
}

function Update-BaseWorktree([string]$Commit) {
    $sha = (& git -C $Repo rev-parse --verify "$Commit^{commit}").Trim()
    if ($LASTEXITCODE) { throw "no commit $Commit" }
    $wt = Join-Path $Shots "base"
    & git -C $Repo worktree prune | Out-Null
    if (-not (Test-Path (Join-Path $wt ".git"))) {
        if (Test-Path $wt) { Remove-Item -Recurse -Force $wt }
        & git -C $Repo worktree add --detach $wt $sha | Out-Null
        if ($LASTEXITCODE) { throw "git worktree add failed" }
    } else {
        & git -C $wt checkout --detach --force $sha | Out-Null
        if ($LASTEXITCODE) { throw "git checkout $sha failed in $wt" }
        & git -C $wt clean -fdq | Out-Null
    }
    $top = (& git -C $wt rev-parse --show-toplevel).Trim()
    if ((Resolve-Path $top).Path -ne (Resolve-Path $wt).Path) { throw "$wt is not its own worktree" }
    return @{ Tree = $wt; Sha = $sha }
}

function Restore-Library {
    # The originals as the fixture made them: the writeback pass changes them in place. Without the pristine copy
    # there is nothing to restore from, and the extra-file pass below would delete the whole library: refuse.
    if (-not (Test-Path $Pristine)) { throw "$Pristine is missing; delete $Library and run again" }
    foreach ($file in Get-ChildItem -Recurse -File $Pristine) {
        $rel = $file.FullName.Substring($Pristine.Length)
        $live = Join-Path $Library $rel
        if (-not (Test-Path $live)) { throw "library file $live is gone; delete $Library and run again" }
        $liveItem = Get-Item $live
        $same = ($liveItem.Length -eq $file.Length) -and
            ((Get-FileHash $live).Hash -eq (Get-FileHash $file.FullName).Hash)
        if (-not $same) {
            [System.IO.File]::WriteAllBytes($live, [System.IO.File]::ReadAllBytes($file.FullName))
        }
        if ($liveItem.LastWriteTimeUtc -ne $file.LastWriteTimeUtc) { $liveItem.LastWriteTimeUtc = $file.LastWriteTimeUtc }
    }
    # Files the app added beside the originals (the temp files of the writes).
    foreach ($extra in Get-ChildItem -Recurse -File $Library) {
        $rel = $extra.FullName.Substring($Library.Length)
        if (-not (Test-Path (Join-Path $Pristine $rel))) { Remove-Item -Force $extra.FullName }
    }
}

# The fixture's identity: the pristine library's files and the generator. A reference is only comparable with a
# library made the same way, so a comparison checks this against the reference's manifest.
function Get-FixtureHash {
    $lines = @(foreach ($file in Get-ChildItem -Recurse -File $Pristine | Sort-Object FullName) {
            $rel = $file.FullName.Substring($Pristine.Length)
            "$rel $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)"
        })
    $lines += "fixture.py $((Get-FileHash $FixturePy -Algorithm SHA256).Hash)"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))
    return [System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::HashData($bytes)) -replace "-", ""
}

function New-Template([string]$Exe, [string]$Dir) {
    if (Test-Path $Dir) { Remove-Item -Recurse -Force $Dir }
    $catalog = Join-Path $Dir "catalog"
    New-Item -ItemType Directory -Force $catalog | Out-Null
    Restore-Library
    # Everything the app would do in the background is done here, so a run starts with nothing left to process:
    # the scan and import, the thumbnails the grid uses, and the borders pass (which the app runs after the import,
    # and which races the screenshot's wait while any file is still due).
    $steps = @(@("scan", $Library), @("import"), @("thumbs", "--height", "120"), @("thumbs", "--height", "220"), @("borders"))
    foreach ($step in $steps) {
        & $Exe --catalog-dir $catalog @step | Out-Host
        if ($LASTEXITCODE) { throw "gallery $($step -join ' ') failed ($LASTEXITCODE)" }
    }
    & $VenvPython $FixturePy views (Join-Path $catalog "catalog.sqlite") | Out-Host
    if ($LASTEXITCODE) { throw "fixture.py views failed" }
    & $VenvPython $FixturePy settled (Join-Path $catalog "catalog.sqlite") | Out-Host
    if ($LASTEXITCODE) { throw "the template catalog has background work left (see above)" }
    Set-Content -Path (Join-Path $Dir "notes.txt") -Value "Not a still image: the Yandex search refuses it before anything is sent."
    return $Dir
}

function Invoke-Shot([string]$Exe, [string]$Side, [string]$Name, [string]$Hooks, [string]$Template, [double]$Seconds) {
    $run = Join-Path $Shots "runs\$Side\$Name"
    if (Test-Path $run) { Remove-Item -Recurse -Force $run }
    New-Item -ItemType Directory -Force $run | Out-Null
    Copy-Item -Recurse (Join-Path $Template "catalog") (Join-Path $run "catalog")
    Copy-Item (Join-Path $Template "notes.txt") $run
    New-Item -ItemType Directory -Force (Join-Path $run "crops") | Out-Null
    Restore-Library
    $hookArgs = $Hooks.Replace("{FIX}", $run).Replace("{RUN}", $run)
    $png = Join-Path $run "shot.png"
    $line = "--screenshot=$png --wait=$Seconds $hookArgs".Trim()
    $env:PG_CATALOG_DIR = Join-Path $run "catalog"
    $env:PG_PREFS = Join-Path $run "prefs.json"
    $proc = Start-Process -FilePath $Exe -ArgumentList $line -PassThru -NoNewWindow
    if (-not $proc.WaitForExit(($Seconds + 110) * 1000)) { $proc.Kill(); return @{ Png = $null; Crops = "TIMEOUT"; Note = "timed out"; Settle = "" } }
    $crops = (Get-ChildItem -File (Join-Path $run "crops") | Sort-Object Name | ForEach-Object {
            "$($_.Name) $((Get-FileHash $_.FullName -Algorithm SHA256).Hash)"
        }) -join "`n"
    # The app's own account of the save: "settled after …", or "timed out …" with what kept the window busy.
    $settle = (Get-Content "$png.settle.txt" -ErrorAction SilentlyContinue) -join " "
    if (-not (Test-Path $png)) { return @{ Png = $null; Crops = $crops; Note = "exit $($proc.ExitCode), no screenshot"; Settle = $settle } }
    return @{ Png = $png; Crops = $crops; Note = ""; Settle = $settle }
}

# One side of the matrix: its template catalog from its own binary, then each scenario.
function Run-Side([string]$Exe, [string]$Side) {
    $template = New-Template $Exe (Join-Path $Shots "sides\$Side")
    $out = @()
    foreach ($name in $Scenarios.Keys) {
        Write-Host "[$Side] $name"
        $shot = Invoke-Shot $Exe $Side $name $Scenarios[$name] $template $Wait
        $out += [pscustomobject]@{ Scenario = $name; Png = $shot.Png; Crops = $shot.Crops; Note = $shot.Note; Settle = $shot.Settle }
    }
    return ,$out
}

# The reference to compare with: the only one recorded, or the one for -Base.
function Resolve-Reference {
    $refs = @(Get-ChildItem -Directory $RefRoot -ErrorAction SilentlyContinue | Where-Object { Test-Path (Join-Path $_.FullName "manifest.json") })
    if ($refs.Count -eq 0) { throw "no reference recorded: run just shots -Record <commit>" }
    if (-not $Base) {
        if ($refs.Count -gt 1) { throw "several references ($(($refs.Name) -join ', ')): pass -Base <commit>" }
        return $refs[0].FullName
    }
    $sha = (& git -C $Repo rev-parse --verify "$Base^{commit}").Trim()
    if ($LASTEXITCODE) { throw "no commit $Base" }
    $dir = Join-Path $RefRoot $sha.Substring(0, 10)
    if (-not (Test-Path (Join-Path $dir "manifest.json"))) { throw "no reference for ${Base}: record it with just shots -Record $Base" }
    return $dir
}

# Write a reference from the base side's results. Every scenario must have settled and produced a screenshot.
function Save-Reference([string]$Sha, [object[]]$Results) {
    foreach ($r in $Results) {
        if (-not $r.Png -or $r.Settle -like "timed out*") { throw "cannot record $($r.Scenario): $($r.Note) $($r.Settle)" }
    }
    $dir = Join-Path $RefRoot $Sha.Substring(0, 10)
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force $dir | Out-Null
    $rows = @()
    foreach ($r in $Results) {
        Copy-Item $r.Png (Join-Path $dir "$($r.Scenario).png")
        $rows += [ordered]@{ scenario = $r.Scenario; crops = $r.Crops; settle = $r.Settle }
    }
    $manifest = [ordered]@{
        commit = $Sha
        recorded = (Get-Date -Format "yyyy-MM-dd")
        fixture = (Get-FixtureHash)
        qt = $qmake
        wait = $Wait
        scenarios = $rows
    }
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $dir "manifest.json") -Encoding utf8
    return $dir
}

# The pixel comparison: System.Drawing, 32-bit ARGB on both sides, read as one int per pixel. Returns the number of
# differing pixels, or the two sizes when they differ; the diff image has the differing pixels red.
Add-Type -AssemblyName System.Drawing
function Compare-Pixels([string]$A, [string]$B, [string]$DiffPath) {
    $ia = [System.Drawing.Bitmap]::new($A)
    $ib = [System.Drawing.Bitmap]::new($B)
    try {
        if ($ia.Width -ne $ib.Width -or $ia.Height -ne $ib.Height) {
            return "size $($ia.Width)x$($ia.Height) vs $($ib.Width)x$($ib.Height)"
        }
        $fmt = [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
        $rect = [System.Drawing.Rectangle]::new(0, 0, $ia.Width, $ia.Height)
        $ca = $ia.Clone($rect, $fmt)
        $cb = $ib.Clone($rect, $fmt)
        try {
            $n = $ia.Width * $ia.Height
            $da = $ca.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadWrite, $fmt)
            $db = $cb.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, $fmt)
            $pa = [int[]]::new($n)
            $pb = [int[]]::new($n)
            [System.Runtime.InteropServices.Marshal]::Copy($da.Scan0, $pa, 0, $n)
            [System.Runtime.InteropServices.Marshal]::Copy($db.Scan0, $pb, 0, $n)
            $cb.UnlockBits($db)
            $count = 0
            $red = [int]0xFFFF0000
            for ($i = 0; $i -lt $n; $i++) {
                if ($pa[$i] -ne $pb[$i]) { $count++; $pa[$i] = $red }
            }
            [System.Runtime.InteropServices.Marshal]::Copy($pa, 0, $da.Scan0, $n)
            $ca.UnlockBits($da)
            if ($count -gt 0) { $ca.Save($DiffPath, [System.Drawing.Imaging.ImageFormat]::Png) }
            return "$count"
        } finally {
            $ca.Dispose()
            $cb.Dispose()
        }
    } finally {
        $ia.Dispose()
        $ib.Dispose()
    }
}

# 1. Python packages for the fixture (a venv under target\shots: the global Python is not changed).
if (-not (Test-Path $VenvPython)) {
    & $Python -m venv (Join-Path $Shots "venv") | Out-Host
    if ($LASTEXITCODE) { throw "venv failed (is $Python Python 3.12?)" }
}
& $VenvPython -m pip install --quiet -r $Requirements | Out-Host
if ($LASTEXITCODE) { throw "pip install failed" }

# 2. The binaries: the working tree's always; the base's only when recording, and the reference is read otherwise.
$currentExe = Build-Gallery $Repo $null
$refDir = $null
$manifest = $null
if ($Record) {
    $worktree = Update-BaseWorktree $Record
    $baseExe = Build-Gallery $worktree.Tree (Join-Path $Shots "base-target")
} elseif (-not $Self) {
    $refDir = Resolve-Reference
    $manifest = Get-Content (Join-Path $refDir "manifest.json") -Raw | ConvertFrom-Json
    if ($manifest.qt -ne $qmake) { Write-Warning "the reference was recorded with Qt at $($manifest.qt); this tree uses $qmake" }
}

# 3. The library, made once, and its pristine copy for Restore-Library.
if (-not (Test-Path $Library)) {
    & $VenvPython $FixturePy library $Library | Out-Host
    if ($LASTEXITCODE) { throw "fixture.py library failed" }
}
if (-not (Test-Path $Pristine)) {
    Copy-Item -Recurse $Library $Pristine
}
$fixtureHash = Get-FixtureHash
if ($manifest -and $manifest.fixture -ne $fixtureHash) {
    throw "the fixture changed since $($manifest.commit.Substring(0, 10)) was recorded; re-record it with just shots -Record $($manifest.commit)"
}

if ($Record) {
    # The reference: the base commit's own screenshots of the matrix, recorded once.
    $baseResults = Run-Side $baseExe "base"
    $dir = Save-Reference $worktree.Sha $baseResults
    Write-Host "recorded $($baseResults.Count) scenarios of $($worktree.Sha.Substring(0, 10)) in $dir"
    exit 0
}

# 4. The sides to compare: the base is the reference, or the working tree itself with -Self.
if ($Self) {
    $baseLabel = "working tree (-Self)"
    $baseResults = Run-Side $currentExe "base"
} else {
    $baseLabel = "$($manifest.commit.Substring(0, 10)) (recorded $($manifest.recorded))"
    $baseResults = foreach ($name in $Scenarios.Keys) {
        $ref = $manifest.scenarios | Where-Object { $_.scenario -eq $name }
        $png = Join-Path $refDir "$name.png"
        if ($ref -and (Test-Path $png)) {
            [pscustomobject]@{ Scenario = $name; Png = $png; Crops = $ref.crops; Note = ""; Settle = $ref.settle }
        } else {
            [pscustomobject]@{ Scenario = $name; Png = $null; Crops = ""; Note = "not in the reference"; Settle = "" }
        }
    }
}
$currentResults = Run-Side $currentExe "current"

# 5. The comparison, scenario by scenario.
$diffDir = Join-Path $Shots "diff"
if (Test-Path $diffDir) { Remove-Item -Recurse -Force $diffDir }
New-Item -ItemType Directory -Force $diffDir | Out-Null
$report = @()
$failed = 0
foreach ($name in $Scenarios.Keys) {
    $b = $baseResults | Where-Object { $_.Scenario -eq $name }
    $c = $currentResults | Where-Object { $_.Scenario -eq $name }
    if (-not $b.Png -or -not $c.Png) {
        $status = "MISSING"; $detail = "base: $($b.Note) current: $($c.Note)"
    } elseif ($b.Settle -like "timed out*" -or $c.Settle -like "timed out*") {
        # Saved at the cap without settling: the picture may be mid-change, so it is not compared.
        $status = "UNSETTLED"; $detail = "base: $($b.Settle) current: $($c.Settle)"
    } else {
        $count = Compare-Pixels $b.Png $c.Png (Join-Path $diffDir "$name.png")
        $cropsSame = ($b.Crops -eq $c.Crops)
        if ($count -eq "0" -and $cropsSame) { $status = "same"; $detail = "" }
        elseif ($count -ne "0" -and $count -notmatch "^\d+$") { $status = "SIZE"; $detail = $count }
        else {
            $status = "DIFFERENT"
            $detail = "pixels: $count"
            if (-not $cropsSame) { $detail += "; crops differ" }
        }
    }
    # A declared difference is reported as such; one that no longer differs means the declaration is stale.
    if ($expectList -contains $name) {
        if ($status -eq "DIFFERENT") { $status = "DECLARED"; $detail += " (declared with -Expect)" }
        elseif ($status -eq "same") { $status = "NOT-SEEN"; $detail = "declared to differ, but identical" }
    }
    if ($status -notin @("same", "DECLARED")) { $failed++ }
    $report += [pscustomobject]@{ Scenario = $name; Result = $status; Detail = $detail }
}

$table = $report | Format-Table -AutoSize | Out-String -Width 200
$summary = "base: $baseLabel`ncurrent: working tree`n$table"
Set-Content -Path (Join-Path $Shots "report.txt") -Value $summary
Write-Host $summary
if ($failed -gt 0) {
    Write-Host "$failed of $($Scenarios.Count) scenarios differ or did not run; diffs in $diffDir"
    exit 1
}
Write-Host "all $($Scenarios.Count) scenarios as expected"
