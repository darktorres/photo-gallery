<#
.SYNOPSIS
The few-owed path against the sorted path, by owed count: where the join's two paths cross (`FEW_ENTRIES_PER_THREAD`).

.DESCRIPTION
For each -Workers count and each -Owed count, on fresh copies of -Catalog with that many pictures owed (`set_owed.py`),
runs `pairs_probe` with the few-owed path forced (`PG_PROBE_FEW` past any count) and with the sorted path forced
(`PG_PROBE_FEW=0`), alternately, -Reps times each, and prints the median join (ms) and cycles (M) of each path. The
background CPU load is printed with each line; a run is not left out for it.

.EXAMPLE
tools/profiling/pairing/few_sweep.ps1 -Catalog D:\scratch\pairing\catalog.sqlite -Workers 12 -Owed 1000,2000,4000
#>
param(
    [Parameter(Mandatory)] [string] $Catalog,
    [int[]] $Workers = @(1, 12),
    [Parameter(Mandatory)] [int[]] $Owed,
    [int] $Reps = 4,
    [string] $Probe = (Join-Path $PSScriptRoot '..\..\..\target\release\examples\pairs_probe.exe'),
    [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) 'pg_pairing_few')
)
$ErrorActionPreference = 'Stop'
$setOwed = Join-Path $PSScriptRoot 'set_owed.py'
New-Item -ItemType Directory $Work -Force | Out-Null
$copy = Join-Path $Work 'catalog.sqlite'
function Median([double[]] $v) {
    $s = $v | Sort-Object
    $n = $s.Count
    if ($n % 2) { $s[($n - 1) / 2] } else { ($s[$n / 2 - 1] + $s[$n / 2]) / 2 }
}
foreach ($w in $Workers) {
    foreach ($n in $Owed) {
        $runs = @{ few = @(); sorted = @() }
        foreach ($rep in 1..$Reps) {
            # Alternate which path runs first.
            $order = if ($rep % 2) { @('few', 'sorted') } else { @('sorted', 'few') }
            foreach ($path in $order) {
                Copy-Item $Catalog $copy -Force
                Remove-Item "$copy-wal", "$copy-shm" -ErrorAction SilentlyContinue
                python -I $setOwed $copy $n | Out-Null
                $env:PG_PROBE_KEEP_OWED = '1'
                $env:PG_PROBE_WORKERS = "$w"
                $env:PG_PROBE_FEW = if ($path -eq 'few') { '1000000000' } else { '0' }
                $line = (& $Probe $Work) -join ' '
                $join = [double][regex]::Match($line, '\bjoin (\d+)').Groups[1].Value
                $cycles = [double][regex]::Match($line, '\bcycles (\d+)').Groups[1].Value
                $runs[$path] += , @($join, $cycles)
            }
        }
        $cpu = (Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average
        $f = $runs.few; $s = $runs.sorted
        '{0,2} workers {1,6} owed: few join {2,5} ms {3,6} M | sorted join {4,5} ms {5,6} M | cpu {6}%' -f $w, $n,
            (Median ($f | ForEach-Object { $_[0] })), (Median ($f | ForEach-Object { $_[1] })),
            (Median ($s | ForEach-Object { $_[0] })), (Median ($s | ForEach-Object { $_[1] })), $cpu
    }
}
Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_WORKERS, Env:PG_PROBE_FEW -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $Work -Recurse -Force
