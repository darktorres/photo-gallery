# The machine's processor load in percent, for the pairing sweeps (sweep.ps1, few_sweep.ps1, keep.ps1), which dot-source
# this file: one counter (_Total of the processor's performance data), so their cpu columns measure the same thing.

function Get-CpuLoad {
    # Inside the function only, so the dot-sourcing script's own settings do not change.
    Set-StrictMode -Version Latest
    $ErrorActionPreference = 'Stop'
    (Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'").PercentProcessorTime
}
