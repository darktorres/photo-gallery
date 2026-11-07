# Fails when a lint tool on PATH is not the version CI installs (.github/workflows/ci.yml). `just lint` runs this first, so a
# local lint cannot pass on a different ruff, markdownlint, cargo-deny or cargo-machete than CI's. The Justfile passes the
# pins (its variables, which CI's own pins match).
param(
    [Parameter(Mandatory)] [string] $Ruff,
    [Parameter(Mandatory)] [string] $Markdownlint,
    [Parameter(Mandatory)] [string] $CargoDeny,
    [Parameter(Mandatory)] [string] $CargoMachete
)

$ErrorActionPreference = 'Stop'

# Each tool prints its version as its last word: "ruff 0.17.3", "0.50.0", "cargo-deny 0.20.2", "0.9.2".
$tools = @(
    @{ Exe = 'ruff'; Want = $Ruff; Fix = 'pip install ruff==' + $Ruff },
    @{ Exe = 'markdownlint'; Want = $Markdownlint; Fix = 'npm install --global markdownlint-cli@' + $Markdownlint },
    @{ Exe = 'cargo-deny'; Want = $CargoDeny; Fix = 'cargo install --locked cargo-deny --version ' + $CargoDeny },
    @{ Exe = 'cargo-machete'; Want = $CargoMachete; Fix = 'cargo install --locked cargo-machete --version ' + $CargoMachete }
)

$problems = @()
foreach ($tool in $tools) {
    if (-not (Get-Command $tool.Exe -ErrorAction SilentlyContinue)) {
        $problems += "$($tool.Exe) is not on PATH: install $($tool.Want) with: $($tool.Fix)"
        continue
    }
    $have = ((& $tool.Exe --version) | Out-String).Trim() -split '\s+' | Select-Object -Last 1
    if ($have -ne $tool.Want) {
        $problems += "$($tool.Exe) is $have, CI uses $($tool.Want): install it with: $($tool.Fix)"
    }
}

if ($problems.Count -gt 0) {
    throw ("lint tools do not match CI's versions:`n  " + ($problems -join "`n  "))
}
Write-Output "lint tools match CI: ruff $Ruff, markdownlint $Markdownlint, cargo-deny $CargoDeny, cargo-machete $CargoMachete"
