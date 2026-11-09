# Builds libjpeg-turbo (JPEG 8.0 API, static, the dynamic CRT) once, for both the static Qt's JPEG plugin and the app's
# turbojpeg. One archive in the process: Qt's qjpeg and the app's turbojpeg must not carry two libjpeg copies (their
# structs differ in size, and the link resolves both to one of them). The reasons are in docs/STATIC_BUILD.md.
#
#   pwsh -File tools/build-jpeg-turbo.ps1                          # into D:\PhotoGallery\static\jpeg-turbo
#   pwsh -File tools/build-jpeg-turbo.ps1 -Work D:\scratch\jpeg    # a clean run, into another folder
#
# Outputs: <Work>\prefix (headers and lib, for Qt's -system-libjpeg), and <Work>\link\turbojpeg.lib, the same archive
# under the name turbojpeg-sys links for TURBOJPEG_SOURCE=explicit (TURBOJPEG_LIB_DIR, TURBOJPEG_INCLUDE_DIR).

param(
    [string]$Work = 'D:\PhotoGallery\static\jpeg-turbo',
    [string]$BuildTools = 'C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools',
    [int]$Jobs = [Environment]::ProcessorCount
)

$ErrorActionPreference = 'Stop'

# ---- Pins. Change them here, and record the reason in docs/STATIC_BUILD.md. --------------------------------------
# The libjpeg-turbo sources are inside the rust-turbojpeg fork at this commit (turbojpeg-sys/libjpeg-turbo), the same
# commit the root Cargo.toml pins for turbojpeg-sys, so the app's and Qt's libjpeg come from one source.
$ForkUrl    = 'https://github.com/darktorres/rust-turbojpeg'
$ForkCommit = '945d853b47a76f98fd97ac5ff43830997712caa2'
$Toolset    = '14.44.35207'                                  # MSVC toolset (vcvars64 -vcvars_ver)

# ---- Helpers ----------------------------------------------------------------------------------------------------
function Invoke-Checked([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

function Import-MsvcEnvironment {
    $vcvars = Join-Path $BuildTools 'VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path $vcvars)) { throw "vcvars64.bat not found: $vcvars (Visual Studio Build Tools, see the pins)" }
    $dump = & cmd.exe /d /c "`"$vcvars`" -vcvars_ver=$Toolset >nul 2>nul && set"
    if ($LASTEXITCODE -ne 0) { throw "vcvars64.bat failed (exit $LASTEXITCODE)" }
    foreach ($line in $dump) {
        $i = $line.IndexOf('=')
        if ($i -gt 0) { Set-Item -Path "env:$($line.Substring(0, $i))" -Value $line.Substring($i + 1) }
    }
    $cl = (Get-Command cl.exe -ErrorAction Stop).Source
    if ($cl -notlike "*\$($Toolset)\*") { throw "cl.exe is not toolset ${Toolset}: $cl" }
}

# ---- Run --------------------------------------------------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $Work, "$Work\logs" | Out-Null
$Work = (Resolve-Path $Work).Path
$Src = Join-Path $Work 'src'
$Build = Join-Path $Work 'build'
$Prefix = Join-Path $Work 'prefix'
$Link = Join-Path $Work 'link'

Import-MsvcEnvironment

# The source: the fork at the pinned commit, by hash (a fresh folder each run, so the build sees only that commit).
if (Test-Path "$Src\.git") { Invoke-Checked 'git clean' { & git -C $Src clean -fdxq } }
else {
    New-Item -ItemType Directory -Force -Path $Src | Out-Null
    Invoke-Checked 'git init' { & git -C $Src init -q }
    # Git for Windows converts line endings (core.autocrlf=true by default), so the headers would differ by machine.
    Invoke-Checked 'git config' { & git -C $Src config core.autocrlf false }
}
$fetched = $false
for ($attempt = 1; $attempt -le 3 -and -not $fetched; $attempt++) {
    & git -C $Src fetch -q --depth 1 $ForkUrl $ForkCommit *> "$Work\logs\git-fetch.log"
    if ($LASTEXITCODE -eq 0) { $fetched = $true }
}
if (-not $fetched) { throw "git fetch failed after 3 attempts: see $Work\logs\git-fetch.log" }
Invoke-Checked 'git checkout' { & git -C $Src checkout -q -f $ForkCommit }
$libjpegSrc = Join-Path $Src 'turbojpeg-sys\libjpeg-turbo'
if (-not (Test-Path (Join-Path $libjpegSrc 'CMakeLists.txt'))) { throw "libjpeg-turbo sources not found in $libjpegSrc" }

# Clean output folders, so each run starts empty.
foreach ($dir in $Build, $Prefix, $Link) {
    if (Test-Path $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
}
New-Item -ItemType Directory -Force -Path $Build | Out-Null
Set-Location $Build

# The configure line. JPEG8: the IJG 8.0 API (JPEG_LIB_VERSION 80), which Qt's libjpeg expects. WITH_CRT_DLL: the dynamic
# CRT (/MD), as the app and Qt are. WITH_TURBOJPEG: the TurboJPEG API the app uses (turbojpeg-static.lib).
$cmakeArgs = @(
    '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release',
    "-DCMAKE_INSTALL_PREFIX=$Prefix", '-DCMAKE_INSTALL_LIBDIR=lib',
    '-DENABLE_SHARED=OFF', '-DENABLE_STATIC=ON',
    '-DWITH_JPEG8=ON', '-DWITH_CRT_DLL=ON', '-DWITH_TURBOJPEG=ON',
    $libjpegSrc
)
Invoke-Checked 'cmake configure' { & cmake @cmakeArgs *> "$Work\logs\configure.log" }
Invoke-Checked 'cmake build' { & cmake --build . --parallel $Jobs *> "$Work\logs\build.log" }
Invoke-Checked 'cmake install' { & cmake --install . *> "$Work\logs\install.log" }

# The app's link name (turbojpeg-sys links `turbojpeg` in TURBOJPEG_LIB_DIR): the same file, under that name.
New-Item -ItemType Directory -Force -Path $Link | Out-Null
Copy-Item "$Prefix\lib\turbojpeg-static.lib" "$Link\turbojpeg.lib"

# The JPEG API version the headers say, checked: Qt's configure and the app both depend on it.
$jconfig = Get-Content "$Prefix\include\jconfig.h" -Raw
if ($jconfig -notmatch '#define\s+JPEG_LIB_VERSION\s+80') { throw "jconfig.h does not have JPEG_LIB_VERSION 80" }

$manifest = [ordered]@{
    fork_url     = $ForkUrl
    fork_commit  = $ForkCommit
    msvc_toolset = $Toolset
    configure    = $cmakeArgs[0..($cmakeArgs.Count - 2)]
    libraries    = @(
        @{ path = "$Prefix\lib\turbojpeg-static.lib"; sha256 = (Get-FileHash "$Prefix\lib\turbojpeg-static.lib" -Algorithm SHA256).Hash },
        @{ path = "$Prefix\lib\jpeg-static.lib"; sha256 = (Get-FileHash "$Prefix\lib\jpeg-static.lib" -Algorithm SHA256).Hash }
    )
}
$manifest | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 "$Work\manifest.json"
"done: $Work\manifest.json"
