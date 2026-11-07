@echo off
rem The Windows side of docs/JPEG_DECODERS.md: GPU decoders, the vendored libjpeg-turbo, and the harness.
rem   eval\jpeg-decoders\win-build.cmd [turbo jpeggpu gpujpeg parajpeg debesheedas harness]   (default: all)
rem Sources go to %JPEG_EVAL%\src at pinned commits (patches\*.patch applied on a fresh clone), builds to
rem %JPEG_EVAL%\build. Needs win-env.cmd's toolchain: VS Build Tools, the CUDA toolkit (CUDA_PATH), CMake, Ninja,
rem LLVM's clang-cl (LLVM_DIR, default %ProgramFiles%\LLVM). NASM is not set up here: it must already be on PATH.
setlocal
call "%~dp0win-env.cmd" || exit /b 1
set "HERE=%~dp0"
set "REPO=%~dp0..\.."
if not exist "%JPEG_EVAL%\src" mkdir "%JPEG_EVAL%\src"
if not exist "%JPEG_EVAL%\build" mkdir "%JPEG_EVAL%\build"
set "STEPS=%*"
if "%STEPS%"=="" set "STEPS=turbo jpeggpu gpujpeg parajpeg debesheedas harness"
for %%s in (%STEPS%) do (
  call :%%s || (echo %%s failed & exit /b 1)
)
exit /b 0

:clone
rem name url commit: shallow clone with LF line endings (as upstream), then the local patch if there is one
if exist "%JPEG_EVAL%\src\%~1" exit /b 0
git -c core.autocrlf=false clone -q "%~2" "%JPEG_EVAL%\src\%~1" || exit /b 1
git -C "%JPEG_EVAL%\src\%~1" -c core.autocrlf=false checkout -q %~3 || exit /b 1
if exist "%HERE%patches\%~1.patch" git -C "%JPEG_EVAL%\src\%~1" apply "%HERE%patches\%~1.patch" || exit /b 1
exit /b 0

:turbo
rem The app's vendored libjpeg-turbo 3.1.0, static with SIMD: the reference and CPU baseline
cmake -G Ninja -S "%REPO%\vendor\turbojpeg-sys\libjpeg-turbo" -B "%JPEG_EVAL%\build\libjpeg-turbo" -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=cl -DENABLE_SHARED=OFF -DWITH_TURBOJPEG=ON "-DCMAKE_INSTALL_PREFIX=%JPEG_EVAL%\deps\libjpeg-turbo" >nul || exit /b 1
cmake --build "%JPEG_EVAL%\build\libjpeg-turbo" -j 4 || exit /b 1
cmake --install "%JPEG_EVAL%\build\libjpeg-turbo" >nul
exit /b

:jpeggpu
call :clone jpeggpu https://github.com/nolmoonen/jpeggpu 3ae7ac0dbc6e1b047abaa67badb6378baa7d39cb || exit /b 1
cmake -G Ninja -S "%JPEG_EVAL%\src\jpeggpu" -B "%JPEG_EVAL%\build\jpeggpu" -DCMAKE_BUILD_TYPE=Release %CUDA_MSVC_FLAGS% "-DCMAKE_CXX_FLAGS=/Zc:preprocessor /EHsc" >nul || exit /b 1
cmake --build "%JPEG_EVAL%\build\jpeggpu" -j 4
exit /b

:gpujpeg
call :clone GPUJPEG https://github.com/CESNET/GPUJPEG 37ab4810657fafeedad8c7a3c4546ea4ef5eb363 || exit /b 1
cmake -G Ninja -S "%JPEG_EVAL%\src\GPUJPEG" -B "%JPEG_EVAL%\build\GPUJPEG" -DCMAKE_BUILD_TYPE=Release %CUDA_MSVC_FLAGS% -DBUILD_SHARED_LIBS=ON -DCMAKE_DISABLE_FIND_PACKAGE_OpenGL=ON -DCMAKE_DISABLE_FIND_PACKAGE_GLEW=ON -DCMAKE_DISABLE_FIND_PACKAGE_GLUT=ON -DCMAKE_DISABLE_FIND_PACKAGE_GLFW=ON "-DCMAKE_C_FLAGS=/Zc:preprocessor" "-DCMAKE_CXX_FLAGS=/Zc:preprocessor /EHsc" >nul || exit /b 1
cmake --build "%JPEG_EVAL%\build\GPUJPEG" -j 4
exit /b

:parajpeg
rem clang-cl for its C++ (GCC/Clang attributes and builtins), nvcc on cl for the .cu; the CUDA build has the CPU
rem backend too. __builtin_cpu_supports needs clang's builtins runtime, which lld-link doesn't add by itself.
call :clone parajpeg https://github.com/anniigr/parajpeg f45081d7f75fdab809e7170c15fd806f42deb375 || exit /b 1
for /d %%v in ("%LLVM_DIR%\lib\clang\*") do set "LIB=%%v\lib\windows;%LIB%"
cmake -G Ninja -S "%JPEG_EVAL%\src\parajpeg" -B "%JPEG_EVAL%\build\parajpeg-cuda" -DCMAKE_BUILD_TYPE=Release "-DCMAKE_C_COMPILER=%CLANG_CL%" "-DCMAKE_CXX_COMPILER=%CLANG_CL%" "-DCMAKE_CXX_FLAGS=/EHsc -O2" -DCMAKE_EXE_LINKER_FLAGS=clang_rt.builtins-x86_64.lib -DPARAJPEG_WITH_CUDA=ON -DCMAKE_CUDA_HOST_COMPILER=cl -DCMAKE_CUDA_FLAGS=-Xcompiler=/Zc:preprocessor -DPARAJPEG_BUILD_TESTS=OFF -DPARAJPEG_BUILD_BENCH=OFF -DPARAJPEG_BUILD_TOOLS=ON >nul || exit /b 1
rem the fuzz replay tool links the kernels' CPU emulator beside the kernels (duplicate symbols on Windows): skip it
cmake --build "%JPEG_EVAL%\build\parajpeg-cuda" -j 4 --target parajpeg pjdec
exit /b

:debesheedas
rem debesheedas/GPU-JPEG-Decoder (a course project): its Makefile's one nvcc line, for this GPU's compute capability
call :clone GPU-JPEG-Decoder https://github.com/debesheedas/GPU-JPEG-Decoder 61cfb3726691fe2436a84696d6a977d5bd8c6123 || exit /b 1
if not exist "%JPEG_EVAL%\build\debesheedas" mkdir "%JPEG_EVAL%\build\debesheedas"
pushd "%JPEG_EVAL%\src\GPU-JPEG-Decoder\cuda-decoder"
nvcc -O3 -std=c++17 -arch=sm_%CUDA_ARCH% --relocatable-device-code true -Xcompiler "/Zc:preprocessor /EHsc /O2" -o "%JPEG_EVAL%\build\debesheedas\decoder.exe" main.cu src/parser.cu src/huffmanTree.cpp utils/stream.cpp utils/utils.cu
set "RC=%ERRORLEVEL%"
popd
exit /b %RC%

:harness
set "S=%JPEG_EVAL%\src"
set "B=%JPEG_EVAL%\build"
cmake -G Ninja -S "%HERE%harness" -B "%B%\harness" -DCMAKE_BUILD_TYPE=Release -DCMAKE_CXX_COMPILER=cl -DWITH_CUDA=ON "-DTURBO_INCLUDE=%JPEG_EVAL%\deps\libjpeg-turbo\include" "-DTURBO_LIB=%JPEG_EVAL%\deps\libjpeg-turbo\lib\turbojpeg-static.lib" "-DPARAJPEG_SRC=%S%\parajpeg" "-DPARAJPEG_LIB=%B%\parajpeg-cuda\parajpeg.lib" "-DJPEGGPU_SRC=%S%\jpeggpu" "-DJPEGGPU_BUILD=%B%\jpeggpu" "-DGPUJPEG_SRC=%S%\GPUJPEG" "-DGPUJPEG_BUILD=%B%\GPUJPEG" >"%B%\harness.configure.log" 2>&1 || (type "%B%\harness.configure.log" & exit /b 1)
cmake --build "%B%\harness" -j 4 || exit /b 1
rem next to its DLLs: GPUJPEG's, and CUDA's (nvJPEG, cudart) from the toolkit
copy /y "%B%\GPUJPEG\gpujpeg.dll" "%B%\harness\" >nul
exit /b
