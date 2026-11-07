@echo off
rem Build environment for the GPU decoder builds (docs/JPEG_DECODERS.md "To rerun"): MSVC from the VS Build Tools,
rem the CUDA toolkit and Ninja. Keys from the repo's .env are loaded when the environment does not already set them,
rem so PG_ROOT, LLVM_DIR, CUDA_ARCH and JPEG_EVAL can live there. CUDA_PATH is read from the environment: NVIDIA's
rem installer sets it, and a shell started before that install can get it from .env instead.
rem Usage: call eval\jpeg-decoders\win-env.cmd, then cmake -G Ninja ...
if exist "%~dp0..\..\.env" for /f "usebackq eol=# tokens=1,* delims==" %%a in ("%~dp0..\..\.env") do if not defined %%a set "%%a=%%b"

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo win-env.cmd: vswhere.exe not found in the Visual Studio Installer folder 1>&2
  exit /b 1
)
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -products * -latest -property installationPath`) do set "VSDIR=%%i"
if not defined VSDIR (
  echo win-env.cmd: vswhere found no Visual Studio installation with the C++ tools 1>&2
  exit /b 1
)
rem vcvars64.bat runs vswhere.exe by its bare name, which resolves only with the Installer folder on PATH.
set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
call "%VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1

if not defined CUDA_PATH (
  echo win-env.cmd: CUDA_PATH is not set; NVIDIA's CUDA installer sets it, or set it in .env 1>&2
  exit /b 1
)
set "PATH=%CUDA_PATH%\bin;%CUDA_PATH%\bin\x64;%PATH%"

rem CUDA_ARCH is the compute capability without the dot (7.5 -> 75). Unset, it comes from the first GPU.
if not defined CUDA_ARCH for /f "usebackq tokens=1,2 delims=." %%c in (`nvidia-smi --query-gpu=compute_cap "--format=csv,noheader" 2^>nul`) do if not defined CUDA_ARCH set "CUDA_ARCH=%%c%%d"
if not defined CUDA_ARCH (
  echo win-env.cmd: CUDA_ARCH is not set and nvidia-smi reported no compute capability 1>&2
  exit /b 1
)

rem CUDA 13's CCCL headers need MSVC's conforming preprocessor; pass these to every CMake configure.
set "CUDA_MSVC_FLAGS=-DCMAKE_C_COMPILER=cl -DCMAKE_CXX_COMPILER=cl -DCMAKE_CUDA_ARCHITECTURES=%CUDA_ARCH% -DCMAKE_CUDA_FLAGS=-Xcompiler=/Zc:preprocessor"

rem For code that needs GCC/Clang extensions (parajpeg): clang-cl for C++, nvcc still on cl.
if not defined LLVM_DIR set "LLVM_DIR=%ProgramFiles%\LLVM"
set "CLANG_CL=%LLVM_DIR%\bin\clang-cl.exe"

if not defined JPEG_EVAL if defined PG_ROOT set "JPEG_EVAL=%PG_ROOT%\tools\jpeg-eval"
if not defined JPEG_EVAL (
  echo win-env.cmd: JPEG_EVAL or PG_ROOT must be set, for example in the repo's .env 1>&2
  exit /b 1
)
