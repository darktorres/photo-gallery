<#
.SYNOPSIS
    Runs the pg-library video test under Process Monitor until one run fails (a CI diagnostic for the Windows job).

.DESCRIPTION
    tests/video.rs failed once on the Windows runner with "Access is denied" from create_dir_all, and passed on every
    local run. Each run here is captured on its own, so the capture of a failing run holds only that run's file
    operations. The first failure keeps its capture (video.pml), its log (run-N.log) and the operations that touched
    the scratch folder or were denied (video-filtered.csv), all under target/procmon. Passing captures are deleted.

    Process Monitor is Sysinternals' ProcessMonitor.zip, checked against the SHA-256 below before it runs.

.PARAMETER Runs
    How many runs to make before giving up without a failure.

.PARAMETER TestExe
    Runs this executable instead of the test binary. Only to check the script itself (for example find.exe fails and
    hostname.exe passes).
#>
[CmdletBinding()]
param(
    [int]$Runs = 40,
    [string]$TestExe
)

$ErrorActionPreference = 'Stop'
$pinnedSha256 = '97330813853e0e514a11ee16a47d4c6fe90148fae834649839afb30487cc8b0b'

$out = Join-Path (Get-Location) 'target/procmon'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Get-ChildItem -Path $out -File | Remove-Item -Force

$tool = Join-Path $out 'ProcessMonitor'
$pm = Join-Path $tool 'Procmon64.exe'
if (-not (Test-Path $pm)) {
    $zip = Join-Path $out 'ProcessMonitor.zip'
    Invoke-WebRequest -Uri 'https://download.sysinternals.com/files/ProcessMonitor.zip' -OutFile $zip
    $hash = (Get-FileHash -Path $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $pinnedSha256) { throw "ProcessMonitor.zip has SHA-256 $hash, not the pinned $pinnedSha256" }
    Expand-Archive -Path $zip -DestinationPath $tool -Force
    Remove-Item $zip
}

if (-not $TestExe) {
    $messages = cargo test -p pg-library --test video --locked --no-run --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'the video test did not build' }
    $TestExe = $messages | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.executable } |
        Select-Object -Last 1 -ExpandProperty executable
    if (-not $TestExe) { throw 'cargo named no test binary for pg-library --test video' }
}
"test binary: $TestExe"

function Wait-ProcmonExit {
    while (Get-Process -Name Procmon64 -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 500 }
}

$failedRun = 0
$pml = $null
for ($run = 1; $run -le $Runs; $run++) {
    $pml = Join-Path $out "run-$run.pml"
    # /NoFilter: a filter saved in this user's Process Monitor settings (as on a developer's machine) hides the file
    # operations, which is why the scratch folder's rows are missing without it.
    Start-Process -FilePath $pm -ArgumentList '/AcceptEula', '/Quiet', '/Minimized', '/NoFilter', '/BackingFile', $pml
    Start-Sleep -Seconds 3   # the capture starts with the driver; the test's first file operations come after this
    try {
        & $TestExe *> (Join-Path $out "run-$run.log")
        $code = $LASTEXITCODE
    }
    finally {
        Start-Process -FilePath $pm -ArgumentList '/Terminate' -Wait
        Wait-ProcmonExit
    }
    "run $run : exit $code"
    if ($code -ne 0) { $failedRun = $run; break }
    Remove-Item $pml
}

if ($failedRun -gt 0) {
    $csv = Join-Path $out "run-$failedRun.csv"
    Start-Process -FilePath $pm -ArgumentList '/AcceptEula', '/OpenLog', $pml, '/SaveAs', $csv -Wait
    Wait-ProcmonExit
    $filtered = Join-Path $out 'video-filtered.csv'
    Get-Content -Path $csv -TotalCount 1 | Set-Content -Path $filtered
    Select-String -Path $csv -Pattern 'scratch_video', 'ACCESS DENIED' -SimpleMatch |
        ForEach-Object { $_.Line } | Add-Content -Path $filtered
    Remove-Item $csv
    Rename-Item -Path $pml -NewName 'video.pml'
    # Zero means the capture missed the run's operations (the driver can take a moment to start), so read it with care.
    $scratchRows = @(Select-String -Path $filtered -Pattern 'scratch_video' -SimpleMatch).Count
    "run $failedRun failed: the capture is in target/procmon (video.pml, video-filtered.csv, run-$failedRun.log); $scratchRows operations on scratch_video"
}
else {
    "no run of $Runs failed"
}

if ($env:GITHUB_OUTPUT) {
    "failed=$(if ($failedRun -gt 0) { 'true' } else { 'false' })" | Add-Content -Path $env:GITHUB_OUTPUT
}
