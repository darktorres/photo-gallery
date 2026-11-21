# build.ps1 - builds build\test_encoder.exe natively: nvcc (CUDA 13.4, for this GPU's compute capability) with the MSVC host compiler.
# Run from any PowerShell:  powershell -ExecutionPolicy Bypass -File build.ps1
# The CUDA and Visual Studio paths below are defaults for this machine. The libjpeg-turbo folders default to PG_HOME's
# (the data folder, from the environment; `just` loads it from .env). Each can be overridden with a parameter.

param(
    [string]$CudaRoot  = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.4',
    [string]$VcvarsDir = 'C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build',
    [string]$VsInstallerDir = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer',
    [string]$JpegSrc,
    [string]$JpegBuild,
    [string]$Arch
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

if ((-not $JpegSrc -or -not $JpegBuild) -and -not $env:PG_HOME) {
    throw 'PG_HOME is not set: set it to the data folder, or pass -JpegSrc and -JpegBuild'
}
if (-not $JpegSrc) { $JpegSrc = Join-Path $env:PG_HOME 'tools\jpeg-eval\src\jpegli\third_party\libjpeg-turbo' }
if (-not $JpegBuild) { $JpegBuild = Join-Path $env:PG_HOME 'tools\jpeg-eval\build\libjpeg-turbo' }

# Unless -Arch is given, the compute capability comes as win_env.py takes it: PG_CUDA_ARCH (7.5 is 75), else the
# first GPU's from nvidia-smi.
if (-not $Arch) {
    $cap = $env:PG_CUDA_ARCH
    if (-not $cap) {
        try { $cap = (& nvidia-smi --query-gpu=compute_cap --format=csv,noheader | Select-Object -First 1) } catch { $cap = '' }
    }
    $cap = ("$cap").Trim() -replace '\.', ''
    if (-not $cap) { throw 'no compute capability: set PG_CUDA_ARCH (7.5 is 75) or pass -Arch sm_NN' }
    $Arch = "sm_$cap"
}

$vcvars   = Join-Path $VcvarsDir 'vcvars64.bat'
$nvcc     = Join-Path $CudaRoot 'bin\nvcc.exe'
$required = @(
    $vcvars,
    $nvcc,
    (Join-Path $JpegSrc 'jpeglib.h'),
    (Join-Path $JpegSrc 'turbojpeg.h'),
    (Join-Path $JpegBuild 'jconfig.h'),
    (Join-Path $JpegBuild 'jpeg-static.lib'),
    (Join-Path $JpegBuild 'turbojpeg-static.lib')
)
foreach ($p in $required) {
    if (-not (Test-Path -LiteralPath $p)) { throw "missing required file: $p" }
}

$outDir = Join-Path $here 'build'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$exe = Join-Path $outDir 'test_encoder.exe'

# vcvars64 must run in the same cmd session as nvcc so that cl.exe is on PATH for nvcc's host compile.
# The steps go into a .cmd file so that no quoting passes through PowerShell's native-argument handling.
$script = Join-Path $outDir 'build_test_encoder.cmd'
$lines = @(
    '@echo off',
    "set `"PATH=$VsInstallerDir;%PATH%`"",   # vcvars64 calls vswhere.exe from the Installer folder
    "call `"$vcvars`" >nul",
    'if errorlevel 1 exit /b 1',
    "`"$nvcc`" -O2 -arch=$Arch -std=c++17 -Xcompiler `"/O2 /EHsc /utf-8 /Zc:preprocessor`" -I`"$JpegSrc`" -I`"$JpegBuild`" -o `"$exe`" `"$here\encode_gpu.cu`" `"$here\test_encoder.cpp`" `"$JpegBuild\turbojpeg-static.lib`" `"$JpegBuild\jpeg-static.lib`"",
    'exit /b %errorlevel%'
)
Set-Content -LiteralPath $script -Value $lines -Encoding ASCII
cmd.exe /d /c "`"$script`""
if ($LASTEXITCODE -ne 0) { throw "build failed with exit code $LASTEXITCODE" }

Write-Host "built: $exe"
