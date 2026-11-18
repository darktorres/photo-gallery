# build.ps1 - builds build\test_encoder.exe natively: nvcc (CUDA 13.4, sm_75) with the MSVC host compiler.
# Run from any PowerShell:  powershell -ExecutionPolicy Bypass -File build.ps1
# Paths below are defaults for this machine; each can be overridden with a parameter.

param(
    [string]$CudaRoot  = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.4',
    [string]$VcvarsDir = 'C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build',
    [string]$VsInstallerDir = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer',
    [string]$JpegSrc   = 'D:\PhotoGallery\tools\jpeg-eval\src\jpegli\third_party\libjpeg-turbo',
    [string]$JpegBuild = 'D:\PhotoGallery\tools\jpeg-eval\build\libjpeg-turbo',
    [string]$Arch      = 'sm_75'
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

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
