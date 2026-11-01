@echo off
rem Build environment for the GPU decoder builds (docs/JPEG_DECODERS.md "To rerun"): MSVC from the VS 2026 Build
rem Tools, CUDA 13.4 and Ninja. CUDA_PATH is set here, not inherited, so a shell started before the CUDA install works.
rem Usage: call eval\jpeg-decoders\win-env.cmd, then cmake -G Ninja ...
set "PATH=C:\Program Files (x86)\Microsoft Visual Studio\Installer;%PATH%"
for /f "usebackq delims=" %%i in (`vswhere -products * -latest -property installationPath`) do set "VSDIR=%%i"
call "%VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set "CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.4"
set "PATH=%CUDA_PATH%\bin;%CUDA_PATH%\bin\x64;%PATH%"
rem CUDA 13's CCCL headers need MSVC's conforming preprocessor; pass these to every CMake configure.
set "CUDA_MSVC_FLAGS=-DCMAKE_C_COMPILER=cl -DCMAKE_CXX_COMPILER=cl -DCMAKE_CUDA_ARCHITECTURES=75 -DCMAKE_CUDA_FLAGS=-Xcompiler=/Zc:preprocessor"
rem For code that needs GCC/Clang extensions (parajpeg): clang-cl for C++, nvcc still on cl.
set "CLANG_CL=C:\Program Files\LLVM\bin\clang-cl.exe"
set "JPEG_EVAL=D:\PhotoGallery\tools\jpeg-eval"
