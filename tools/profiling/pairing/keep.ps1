param(
    [Parameter(Mandatory)] [string] $A,
    [Parameter(Mandatory)] [string] $B,
    [Parameter(Mandatory)] [string] $Catalog,
    [Parameter(Mandatory)] [string] $Out,
    [int] $Pairs = 6,
    [int] $Repeat = 1,
    [switch] $KeepTurbo,
    [ValidateSet('bounded', 'no-loss', 'margin')] [string] $Rule = 'bounded',
    [string[]] $Workloads = @('all-w1', 'all-w12', 'few300-w1', 'few300-w12', 'few3000-w1', 'few3000-w12', 'slices4-w12'),
    [hashtable] $EnvA = @{},
    [hashtable] $EnvB = @{}
)
# Paired runs of two pairs_probe builds for the keep rule (keep.py judges them). Each pair runs one workload on both
# builds, one after the other, the order alternating, each on a fresh copy of the catalog (a scratch copy: the probe
# rewrites what is owed). The background CPU load is logged before each run. $EnvA / $EnvB set probe-only switches, to
# compare two designs inside one binary ($A and $B the same file). With $Repeat above 1, each build runs that many
# times in its turn and each measure keeps its least value (another process can only add to a run's time).
#
# The gate is time (keep.py --part time: the join's, the pass's on slices4), under $Rule: 'bounded' (the default, for
# the 6 pairs the plan runs: no proven loss and an upper bound of at most +10%, else no verdict), 'no-loss', or
# 'margin' (+1%, for about 60 pairs).
#
# The measures: the pass's wall clock and its parts (ms), and `cycles`, its CPU cycles over all threads (millions),
# which other processes do not inflate, but which count work moved onto a core's idle second thread as a loss: they
# explain a result and do not gate it.
#
# Workloads: all-wN (every picture owed, N threads), fewK-wN (K owed, their pairs dropped first, the few-owed or the
# sorted path as N decides), slices4-w12 (all owed in four slices of 50,000, so writes overlap joins).
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$base = Join-Path ([IO.Path]::GetTempPath()) 'pg_keep'
if (Test-Path $base) { Remove-Item -LiteralPath $base -Recurse -Force }
New-Item -ItemType Directory $base | Out-Null
$drop = "import sqlite3, sys`nc = sqlite3.connect(sys.argv[1])`nc.execute('PRAGMA foreign_keys = ON')`n" +
    "c.execute('DELETE FROM pdq_pairs WHERE a IN (SELECT file_id FROM pairs_owed) OR b IN (SELECT file_id FROM pairs_owed)')`n" +
    "c.commit()"
$dropPy = Join-Path $base 'drop.py'
Set-Content -LiteralPath $dropPy -Value $drop
# One template catalog per owed count, made once.
$templates = @{}
foreach ($w in $Workloads) {
    if ($w -match '^few(\d+)-') {
        $k = $Matches[1]
        if (-not $templates.ContainsKey($k)) {
            $dir = Join-Path $base "owed$k"
            New-Item -ItemType Directory $dir | Out-Null
            $db = Join-Path $dir 'catalog.sqlite'
            Copy-Item -LiteralPath $Catalog $db
            python -I (Join-Path $here 'set_owed.py') $db $k | Out-Null
            python -I $dropPy $db
            $templates[$k] = $db
        }
    }
}
$run = Join-Path $base 'run'
$parts = 'load', 'join', 'prepare', 'sort', 'scan', 'write', 'commit', 'cycles'
$rows = [System.Collections.Generic.List[object]]::new()
# Turbo off while measuring: the power plan's maximum processor state at 99% (from its own value, restored however the
# runs end), so the clock does not move with the load of other processes. -KeepTurbo leaves it.
$throttle = $null
if (-not $KeepTurbo) {
    $query = (powercfg /query scheme_current sub_processor PROCTHROTTLEMAX) -join "`n"
    $values = [regex]::Matches($query, '0x([0-9a-fA-F]{8})\s*$', 'Multiline') | ForEach-Object { [Convert]::ToInt32($_.Groups[1].Value, 16) }
    # The last two: the AC and the DC index, in that order.
    $throttle = @($values)[-2, -1]
    powercfg /setacvalueindex scheme_current sub_processor PROCTHROTTLEMAX 99
    powercfg /setdcvalueindex scheme_current sub_processor PROCTHROTTLEMAX 99
    powercfg /setactive scheme_current
    "turbo off: maximum processor state 99% (was AC $($throttle[0])%, DC $($throttle[1])%)"
}
try {
for ($p = 0; $p -lt $Pairs; $p++) {
    foreach ($w in $Workloads) {
        if ($w -notmatch '^(all|few(\d+)|slices4)-w(\d+)$') { throw "unknown workload $w" }
        $kind, $owed, $threads = $Matches[1], $Matches[2], $Matches[3]
        $order = if ($p % 2 -eq 0) { 'A', 'B' } else { 'B', 'A' }
        foreach ($build in $order) {
            $row = $null
            for ($k = 0; $k -lt $Repeat; $k++) {
                if (Test-Path $run) { Remove-Item -LiteralPath $run -Recurse -Force }
                New-Item -ItemType Directory $run | Out-Null
                $source = if ($owed) { $templates[$owed] } else { $Catalog }
                Copy-Item -LiteralPath $source (Join-Path $run 'catalog.sqlite')
                Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE -ErrorAction SilentlyContinue
                if ($owed) { $env:PG_PROBE_KEEP_OWED = '1' }
                if ($kind -eq 'slices4') { $env:PG_PROBE_SLICE = '50000' }
                $env:PG_PROBE_WORKERS = $threads
                $extra = if ($build -eq 'A') { $EnvA } else { $EnvB }
                foreach ($e in $extra.Keys) { Set-Item "Env:$e" $extra[$e] }
                $cpu = (Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average
                $exe = if ($build -eq 'A') { $A } else { $B }
                $line = (& $exe $run) -join ' '
                foreach ($e in $extra.Keys) { Remove-Item "Env:$e" -ErrorAction SilentlyContinue }
                $m = [regex]::Match($line, '(\d+) pairs in ([\d.]+) s')
                if (-not $m.Success) { throw "probe output not understood: $line" }
                $one = [ordered]@{ pass = [int][math]::Round([double]::Parse($m.Groups[2].Value, [Globalization.CultureInfo]::InvariantCulture) * 1000) }
                foreach ($n in $parts) { $one[$n] = [int][regex]::Match($line, "\b$n (\d+)").Groups[1].Value }
                if (-not $row) {
                    $row = [ordered]@{ pair = $p; workload = $w; build = $build; cpu = $cpu; pairs = [int]$m.Groups[1].Value }
                    foreach ($n in $one.Keys) { $row[$n] = $one[$n] }
                } else {
                    $row.cpu = [math]::Max($row.cpu, $cpu)
                    foreach ($n in $one.Keys) { $row[$n] = [math]::Min($row[$n], $one[$n]) }
                }
            }
            $rows.Add([pscustomobject]$row)
            "{0} {1} {2} cpu {3}%: pass {4} ms, cycles {5} M, {6} pairs" -f $p, $w, $build, $row.cpu, $row.pass, $row.cycles, $row.pairs
        }
    }
    $rows | Export-Csv -NoTypeInformation -LiteralPath $Out
}
} finally {
    if ($throttle) {
        powercfg /setacvalueindex scheme_current sub_processor PROCTHROTTLEMAX $throttle[0]
        powercfg /setdcvalueindex scheme_current sub_processor PROCTHROTTLEMAX $throttle[1]
        powercfg /setactive scheme_current
        "turbo restored: maximum processor state AC $($throttle[0])%, DC $($throttle[1])%"
    }
}
Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE, Env:PG_PROBE_WORKERS -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $base -Recurse -Force
$margin = if ($Rule -eq 'margin') { '0.01' } else { '0.10' }
python -I (Join-Path $here 'keep.py') $Out --part time --rule $Rule --margin $margin
