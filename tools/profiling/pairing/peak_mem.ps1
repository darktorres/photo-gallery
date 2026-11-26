<#
.SYNOPSIS
The pairing pass's peak memory: peak working set and peak commit (private bytes) of `pairs_probe`, sampled every 15 ms.

.DESCRIPTION
For each -Owed count, on a fresh copy of -Catalog: sets what is owed (`set_owed.py`; 'all' uses `pairs_again`), runs the
probe and samples the process until it exits. The peak commit is the pass's memory figure (the plan that
stated it, `16f5fa6f^:docs/plans/PAIRING_PASS_PLAN.md`, is deleted). 'all' owes every picture through the probe's `pairs_again`, which
reads `analysis` on the catalog's connection before the pass bounds its cache; the count of every picture (`set_owed.py`)
measures the pass alone. -Slice and -CacheKib set the pass's `Settings` (0: the default).

.EXAMPLE
tools/profiling/pairing/peak_mem.ps1 -Catalog D:\scratch\pairing\catalog.sqlite -Owed 0,1,30000,all -Reps 2
#>
param(
    [Parameter(Mandatory)] [string] $Catalog,
    [string[]] $Owed = @('0', '1', '30000', 'all'),
    [int] $Reps = 2,
    [int] $Workers = 12,
    [int] $Slice = 0,
    [int] $CacheKib = 0,
    [string] $Probe = (Join-Path $PSScriptRoot '..\..\..\target\release\examples\pairs_probe.exe'),
    [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) 'pg_pairing_mem')
)
$ErrorActionPreference = 'Stop'
$setOwed = Join-Path $PSScriptRoot 'set_owed.py'
# -Work is scratch these scripts own: it is removed only when it carries this marker, which the script writes into the
# folder it makes. A folder the caller named that lacks it is refused, not deleted.
$scratchMarker = '.pairing-scratch'
function Clear-Work {
    if (-not (Test-Path $Work)) { return }
    if (-not (Test-Path (Join-Path $Work $scratchMarker))) { throw "-Work $Work is not a scratch folder of these scripts: name a folder that does not exist" }
    Remove-Item -LiteralPath $Work -Recurse -Force
}
foreach ($rep in 1..$Reps) {
    foreach ($n in $Owed) {
        Clear-Work
        New-Item -ItemType Directory $Work | Out-Null
        New-Item -ItemType File (Join-Path $Work $scratchMarker) | Out-Null
        Copy-Item -LiteralPath $Catalog (Join-Path $Work 'catalog.sqlite')
        Remove-Item Env:PG_PROBE_KEEP_OWED -ErrorAction SilentlyContinue
        if ($n -ne 'all') {
            python -I $setOwed (Join-Path $Work 'catalog.sqlite') $n | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "set_owed.py failed (exit $LASTEXITCODE): the run would not owe what -Owed says" }
            $env:PG_PROBE_KEEP_OWED = '1'
        }
        if ($Slice -gt 0) { $env:PG_PROBE_SLICE = "$Slice" } else { Remove-Item Env:PG_PROBE_SLICE -ErrorAction SilentlyContinue }
        if ($CacheKib -gt 0) { $env:PG_PROBE_CACHE_KIB = "$CacheKib" } else { Remove-Item Env:PG_PROBE_CACHE_KIB -ErrorAction SilentlyContinue }
        $env:PG_PROBE_WORKERS = "$Workers"
        $stdout = Join-Path $Work 'probe.out'
        $p = Start-Process -FilePath $Probe -ArgumentList @($Work) -NoNewWindow -PassThru -RedirectStandardOutput $stdout
        $ws = 0; $commit = 0
        while (-not $p.HasExited) {
            try {
                $p.Refresh()
                $ws = [Math]::Max($ws, $p.PeakWorkingSet64)
                $commit = [Math]::Max($commit, $p.PrivateMemorySize64)
            } catch { }
            Start-Sleep -Milliseconds 15
        }
        $p.WaitForExit()
        '{0,6} owed rep {1}: peak working set {2,5:N0} MB, peak commit {3,5:N0} MB :: {4}' -f `
            $n, $rep, ($ws / 1MB), ($commit / 1MB), ((Get-Content $stdout -Raw) -replace '\s+', ' ')
    }
}
Clear-Work
Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE, Env:PG_PROBE_CACHE_KIB, Env:PG_PROBE_WORKERS -ErrorAction SilentlyContinue
