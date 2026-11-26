# build.ps1 - builds build\test_encoder.exe natively: nvcc (CUDA 13.4, for this GPU's compute capability) with the MSVC host compiler.
# Run from any PowerShell:  powershell -ExecutionPolicy Bypass -File build.ps1
# The CUDA path below is a default for this machine. Visual Studio is found with vswhere, as win_env.py finds it. The
# libjpeg-turbo folders are the decoder comparison's own build of the app's libjpeg-turbo, under PG_JPEG_EVAL (or
# PG_ROOT\tools\jpeg-eval); `just` loads both from .env. Each can be overridden with a parameter.

param(
    [string]$CudaRoot  = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.4',
    [string]$VcvarsDir,
    [string]$VsInstallerDir,
    [string]$JpegSrc,
    [string]$JpegBuild,
    [string]$Arch
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

$jpegEval = $env:PG_JPEG_EVAL
if (-not $jpegEval -and $env:PG_ROOT) { $jpegEval = Join-Path $env:PG_ROOT 'tools\jpeg-eval' }
if ((-not $JpegSrc -or -not $JpegBuild) -and -not $jpegEval) {
    throw 'PG_JPEG_EVAL or PG_ROOT is not set: set one in .env, or pass -JpegSrc and -JpegBuild'
}
if (-not $JpegSrc) { $JpegSrc = Join-Path $jpegEval 'src\rust-turbojpeg\turbojpeg-sys\libjpeg-turbo' }
if (-not $JpegBuild) { $JpegBuild = Join-Path $jpegEval 'build\libjpeg-turbo' }

if (-not $VsInstallerDir) { $VsInstallerDir = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer' }
if (-not $VcvarsDir) {
    # The newest Visual Studio with the C++ tools, as win_env.py finds it.
    $vswhere = Join-Path $VsInstallerDir 'vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw "vswhere.exe not found in $VsInstallerDir" }
    $vsPath = & $vswhere -products '*' -latest -property installationPath | Select-Object -First 1
    if (-not $vsPath) { throw 'vswhere found no Visual Studio installation with the C++ tools' }
    $VcvarsDir = Join-Path $vsPath 'VC\Auxiliary\Build'
}

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
