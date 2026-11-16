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
$check = @'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
got = [f"{a} {b} {d}" for a, b, d in c.execute("SELECT a, b, distance FROM pdq_pairs ORDER BY a, b")]
want = [w for w in open(sys.argv[2], encoding="utf-8").read().split("\n") if w]
owed = c.execute("SELECT count(*) FROM pairs_owed").fetchone()[0]
print(f"pairs {len(got)} oracle {len(want)} identical {got == want} still owed {owed}")
'@
$drop = @'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("PRAGMA foreign_keys = ON")
n = c.execute("DELETE FROM pdq_pairs WHERE a IN (SELECT file_id FROM pairs_owed) OR b IN (SELECT file_id FROM pairs_owed)").rowcount
c.commit()
print(f"deleted {n} pairs of the owed pictures")
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
    if (Test-Path $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
    New-Item -ItemType Directory $Work | Out-Null
    $checkPy = Join-Path $Work 'check.py'
    $dropPy = Join-Path $Work 'drop.py'
    Set-Content -LiteralPath $checkPy -Value $check
    Set-Content -LiteralPath $dropPy -Value $drop
    $db = Join-Path $Work 'catalog.sqlite'
    Copy-Item -LiteralPath $Catalog $db
    Remove-Item Env:PG_PROBE_KEEP_OWED -ErrorAction SilentlyContinue
    $dropped = ''
    if ($case.owed -ne 'all') {
        python -I $setOwed $db $case.owed | Out-Null
        $dropped = python -I $dropPy $db
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
Remove-Item -LiteralPath $Work -Recurse -Force
if ($failed -gt 0) { throw "$failed of $($cases.Count) cases differ from the oracle" }
"all $($cases.Count) cases identical"
