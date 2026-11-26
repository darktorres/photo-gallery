<#
.SYNOPSIS
The pairing pass's correctness on real data: every path of the pass, each on a fresh copy of a catalog, against an oracle
of its pairs.

.DESCRIPTION
For each case, on a fresh copy of -Catalog in -Work: what is owed is set (every picture, through the probe's
`pairs_again`; or N spread over the ids by `set_owed.py`, with those pictures' pairs deleted first, so the pass has to
make them again), `pairs_probe` runs, and `pdq_pairs` is compared with -Oracle line by line. A case passes when the
pairs are identical and nothing is left owed. The cases are the seven paths a change to the pass must keep: one slice at
12 workers and at 1, slices of 50,000 (the write overlapping the next slice's join), 300 owed (the few-owed path at
12 workers), and 3,000 (at 12 workers and at 1) and 30,000 owed on the sorted path.

-Oracle is a text file, one pair per line, `a b distance`, in (a, b) order: what
`SELECT a, b, distance FROM pdq_pairs ORDER BY a, b` gives on a catalog the reference build paired.

Never point -Catalog at the live catalog: the copies are written, the source only read.

.EXAMPLE
tools/profiling/pairing/oracle.ps1 -Catalog D:\scratch\pairing\catalog.sqlite -Oracle D:\scratch\e2e\after_pairs.txt
#>
param(
    [Parameter(Mandatory)] [string] $Catalog,
    [Parameter(Mandatory)] [string] $Oracle,
    [string] $Probe = (Join-Path $PSScriptRoot '..\..\..\target\release\examples\pairs_probe.exe'),
    [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) 'pg_pairing_oracle')
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
$check = @'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
got = [f"{a} {b} {d}" for a, b, d in c.execute("SELECT a, b, distance FROM pdq_pairs ORDER BY a, b")]
want = [w for w in open(sys.argv[2], encoding="utf-8").read().split("\n") if w]
owed = c.execute("SELECT count(*) FROM pairs_owed").fetchone()[0]
print(f"pairs {len(got)} oracle {len(want)} identical {got == want} still owed {owed}")
'@
$cases = @(
    @{ name = 'all owed, one slice'; owed = 'all'; slice = 0; workers = 12 },
    @{ name = 'all owed, slices of 50000 (write overlap)'; owed = 'all'; slice = 50000; workers = 12 },
    @{ name = 'all owed, one worker'; owed = 'all'; slice = 0; workers = 1 },
    @{ name = '300 owed (few-owed path)'; owed = '300'; slice = 0; workers = 12 },
    @{ name = '3000 owed (sorted path at 12)'; owed = '3000'; slice = 0; workers = 12 },
    @{ name = '3000 owed (sorted path at 1)'; owed = '3000'; slice = 0; workers = 1 },
    @{ name = '30000 owed (sorted path)'; owed = '30000'; slice = 0; workers = 12 }
)
$failed = 0
foreach ($case in $cases) {
    Clear-Work
    New-Item -ItemType Directory $Work | Out-Null
    New-Item -ItemType File (Join-Path $Work $scratchMarker) | Out-Null
    $checkPy = Join-Path $Work 'check.py'
    $dropPy = Join-Path $PSScriptRoot 'drop_owed_pairs.py'
    Set-Content -LiteralPath $checkPy -Value $check
    $db = Join-Path $Work 'catalog.sqlite'
    Copy-Item -LiteralPath $Catalog $db
    Remove-Item Env:PG_PROBE_KEEP_OWED -ErrorAction SilentlyContinue
    $dropped = ''
    if ($case.owed -ne 'all') {
        python -I $setOwed $db $case.owed | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "set_owed.py failed (exit $LASTEXITCODE) on case '$($case.name)': the case would not owe what it names" }
        $dropped = python -I $dropPy $db
        if ($LASTEXITCODE -ne 0) { throw "drop_owed_pairs.py failed (exit $LASTEXITCODE) on case '$($case.name)'" }
        $env:PG_PROBE_KEEP_OWED = '1'
    }
    if ($case.slice -gt 0) { $env:PG_PROBE_SLICE = "$($case.slice)" } else { Remove-Item Env:PG_PROBE_SLICE -ErrorAction SilentlyContinue }
    $env:PG_PROBE_WORKERS = "$($case.workers)"
    $line = (& $Probe $Work) -join ' '
    $result = python -I $checkPy $db $Oracle
    if ($result -notmatch 'identical True still owed 0$') { $failed++ }
    "$($case.name): $dropped | $result"
    "    $($line -replace '^owed .*? in ', 'in ')"
}
Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE, Env:PG_PROBE_WORKERS -ErrorAction SilentlyContinue
Clear-Work
if ($failed -gt 0) { throw "$failed of $($cases.Count) cases differ from the oracle" }
"all $($cases.Count) cases identical"
