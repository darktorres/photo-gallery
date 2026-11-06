# Fetches the pinned FFmpeg build for `just setup`: downloads the archive once (into .deps/downloads), checks its SHA-256
# against the pin, and unpacks it to -Destination. The folder is skipped when its headers are already there. The Justfile
# holds the URL and the hash; CI runs the same recipe.
#
# A download that does not match the hash is deleted and the script fails, so a bad archive is never unpacked or cached.

param(
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][string]$Sha256,
    # Relative to the repository root (the folder above tools/).
    [Parameter(Mandatory)][string]$Destination
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest's progress bar slows a download to a crawl in Windows PowerShell and pwsh alike.
$ProgressPreference = 'SilentlyContinue'

$repo = Split-Path -Parent $PSScriptRoot
$dest = [IO.Path]::GetFullPath((Join-Path $repo $Destination))
$header = Join-Path $dest 'include/libavcodec/avcodec.h'
if (Test-Path -LiteralPath $header -PathType Leaf) {
    Write-Host "FFmpeg is already in ${dest}: nothing to do (delete the folder to fetch it again)."
    exit 0
}

$downloads = Join-Path $repo '.deps/downloads'
$zip = Join-Path $downloads (Split-Path -Leaf ([Uri]$Url).AbsolutePath)
New-Item -ItemType Directory -Force -Path $downloads | Out-Null

function Get-Sha256([string]$Path) {
    (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash
}

# A download left from an earlier run is used if it is intact; otherwise it is fetched again.
if (-not ((Test-Path -LiteralPath $zip) -and ((Get-Sha256 $zip) -ieq $Sha256))) {
    Remove-Item -LiteralPath $zip -ErrorAction SilentlyContinue
    Write-Host "Downloading $Url"
    # The .partial name keeps an interrupted download from looking like a finished one.
    $partial = "$zip.partial"
    Invoke-WebRequest -Uri $Url -OutFile $partial
    Move-Item -LiteralPath $partial -Destination $zip -Force
    $actual = Get-Sha256 $zip
    if ($actual -ine $Sha256) {
        Remove-Item -LiteralPath $zip
        throw "the FFmpeg archive has SHA-256 $actual, but the pin is ${Sha256}: the download was deleted (the Justfile holds the pin)"
    }
}

# Unpack to a scratch folder, then move the one top folder of the archive to the destination.
$extract = Join-Path $repo '.deps/extract'
if (Test-Path -LiteralPath $extract) {
    Remove-Item -LiteralPath $extract -Recurse -Force
}
Expand-Archive -LiteralPath $zip -DestinationPath $extract

$top = @(Get-ChildItem -LiteralPath $extract -Directory)
if ($top.Count -ne 1) {
    throw "the FFmpeg archive has $($top.Count) top folders, expected one"
}
# Only a partial or older copy is replaced: the header check above found no usable build here.
if (Test-Path -LiteralPath $dest) {
    Remove-Item -LiteralPath $dest -Recurse -Force
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null
Move-Item -LiteralPath $top[0].FullName -Destination $dest
Remove-Item -LiteralPath $extract -Recurse -Force

if (-not (Test-Path -LiteralPath $header -PathType Leaf)) {
    throw "the archive has no include/libavcodec/avcodec.h: it is not a shared FFmpeg build"
}
Write-Host "FFmpeg is in $dest."
