<#
.SYNOPSIS
The pairing pass timed over worker counts, on fresh copies of a catalog (docs/reference/PROFILING.md, "Pairing pass").

.DESCRIPTION
Runs `pairs_probe` (a release build: `cargo build --release -p pg-library --example pairs_probe`) on a fresh copy of
-Catalog for every run, interleaving the worker counts so a slow spell of the machine is spread over all of them. Before
each run the machine's CPU load is read; a run that starts above 30% is flagged with `busy` (the machine is shared, and
such runs are left out of the medians). Prints every run, then the median of each part per worker count.

-Owed: 'all' (every eligible picture owes its pairs, `pairs_again`) or a number (that many owe, `set_owed.py`).
-Slice: the slice size (`PG_PROBE_SLICE`); -CacheKib: the pass's page cache (`PG_PROBE_CACHE_KIB`). 0: the default.

.EXAMPLE
tools/profiling/pairing/sweep.ps1 -Catalog D:\scratch\pairing\catalog.sqlite -Workers 1,6,12 -Reps 4
#>
param(
    [Parameter(Mandatory)] [string] $Catalog,
    [int[]] $Workers = @(1, 2, 4, 6, 12),
    [int] $Reps = 4,
    [string] $Owed = 'all',
    [int] $Slice = 0,
    [int] $CacheKib = 0,
    [string] $Probe = (Join-Path $PSScriptRoot '..\..\..\target\release\examples\pairs_probe.exe'),
    [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) 'pg_pairing_sweep')
)
$ErrorActionPreference = 'Stop'
$setOwed = Join-Path $PSScriptRoot 'set_owed.py'
. (Join-Path $PSScriptRoot 'cpu_load.ps1')
# -Work is scratch these scripts own: it is removed only when it carries this marker, which the script writes into the
# folder it makes. A folder the caller named that lacks it is refused, not deleted.
$scratchMarker = '.pairing-scratch'
function Clear-Work {
    if (-not (Test-Path $Work)) { return }
    if (-not (Test-Path (Join-Path $Work $scratchMarker))) { throw "-Work $Work is not a scratch folder of these scripts: name a folder that does not exist" }
    Remove-Item -LiteralPath $Work -Recurse -Force
}
function Get-Median([double[]] $xs) {
    $s = $xs | Sort-Object
    if ($s.Count -eq 0) { return [double]::NaN }
    if ($s.Count % 2) { return $s[[int][Math]::Floor($s.Count / 2)] }
    ($s[$s.Count / 2 - 1] + $s[$s.Count / 2]) / 2
}

$runs = @()
foreach ($rep in 1..$Reps) {
    foreach ($w in $Workers) {
        Clear-Work
        New-Item -ItemType Directory $Work | Out-Null
        New-Item -ItemType File (Join-Path $Work $scratchMarker) | Out-Null
        Copy-Item -LiteralPath $Catalog (Join-Path $Work 'catalog.sqlite')
        Remove-Item Env:PG_PROBE_KEEP_OWED -ErrorAction SilentlyContinue
        if ($Owed -ne 'all') {
            python -I $setOwed (Join-Path $Work 'catalog.sqlite') $Owed | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "set_owed.py failed (exit $LASTEXITCODE): the run would not owe what -Owed says" }
            $env:PG_PROBE_KEEP_OWED = '1'
        }
        if ($Slice -gt 0) { $env:PG_PROBE_SLICE = "$Slice" } else { Remove-Item Env:PG_PROBE_SLICE -ErrorAction SilentlyContinue }
        if ($CacheKib -gt 0) { $env:PG_PROBE_CACHE_KIB = "$CacheKib" } else { Remove-Item Env:PG_PROBE_CACHE_KIB -ErrorAction SilentlyContinue }
        $env:PG_PROBE_WORKERS = "$w"
        $load = Get-CpuLoad
        $line = (& $Probe $Work) -join ' '
        $run = [ordered]@{ rep = $rep; workers = $w; load = $load; busy = ($load -gt 30); line = $line }
        if ($line -match 'in ([0-9.]+) s') { $run.total = [double]$Matches[1] * 1000 }
        foreach ($m in [regex]::Matches($line, '(\w+) ([0-9.]+)(?= |$| ms)')) {
            $name = $m.Groups[1].Value
            if ($name -in 'load', 'join', 'write', 'commit', 'prepare', 'sort', 'scan') { $run[$name] = [double]$m.Groups[2].Value }
        }
        $runs += [pscustomobject]$run
        "rep $rep workers $w cpu $load%$(if ($run.busy) { ' busy' }) :: $line"
    }
}
Clear-Work
Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE, Env:PG_PROBE_CACHE_KIB, Env:PG_PROBE_WORKERS -ErrorAction SilentlyContinue

''
'medians (ms), runs that started busy left out:'
foreach ($w in $Workers) {
    $quiet = @($runs | Where-Object { $_.workers -eq $w -and -not $_.busy })
    $parts = foreach ($p in 'total', 'load', 'join', 'prepare', 'sort', 'scan', 'write', 'commit') {
        $xs = @($quiet | Where-Object { $null -ne $_.$p } | ForEach-Object { $_.$p })
        if ($xs.Count) { '{0} {1:F0}' -f $p, (Get-Median $xs) }
    }
    'workers {0,2} ({1} quiet runs): {2}' -f $w, $quiet.Count, ($parts -join ', ')
}
