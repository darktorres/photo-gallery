<#
.SYNOPSIS
The pairing pass's peak memory: peak working set and peak commit (private bytes) of `pairs_probe`, sampled every 15 ms.

.DESCRIPTION
For each -Owed count, on a fresh copy of -Catalog: sets what is owed (`set_owed.py`; 'all' uses `pairs_again`), runs the
probe and samples the process until it exits. The peak commit is what the plan's memory cap is stated in
(docs/plans/PAIRING_PASS_PLAN.md, Part 2 item 6). 'all' owes every picture through the probe's `pairs_again`, which
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
foreach ($rep in 1..$Reps) {
    foreach ($n in $Owed) {
        if (Test-Path $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
        New-Item -ItemType Directory $Work | Out-Null
        Copy-Item -LiteralPath $Catalog (Join-Path $Work 'catalog.sqlite')
        Remove-Item Env:PG_PROBE_KEEP_OWED -ErrorAction SilentlyContinue
        if ($n -ne 'all') {
            python -I $setOwed (Join-Path $Work 'catalog.sqlite') $n | Out-Null
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
if (Test-Path $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE, Env:PG_PROBE_CACHE_KIB, Env:PG_PROBE_WORKERS -ErrorAction SilentlyContinue
