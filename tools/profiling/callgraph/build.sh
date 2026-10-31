#!/bin/bash
# Builds callgraph.dll (callgraph.cpp) into target/pin with the Pin kit that comes with Intel SDE, from Git Bash:
# Pin 4's wrappers (pinrt/bin/pin-clang-cl++, pin-ld) around LLVM's clang-cl and lld-link, with Pin's own C and C++
# runtime. SDE's mbuild scripts do the same but don't know Visual Studio 18. Needs LLVM (C:\Program Files\LLVM) and
# the Windows SDK's kernel32.lib.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
pin=${PIN_ROOT:-$(cygpath -u "$LOCALAPPDATA")/Programs/sde/pinkit}
out=$here/../../../target/pin
mkdir -p "$out"
export PATH="/c/Program Files/LLVM/bin:$PATH"
clang_inc=$(ls -d "/c/Program Files/LLVM/lib/clang/"*/include | sort -V | tail -1)
sdk_lib=$(ls -d "/c/Program Files (x86)/Windows Kits/10/Lib/"*/um/x64 | sort -V | tail -1)
w() { cygpath -m "$1"; }
# Arguments that embed a Windows path after a flag are left alone by MSYS; the wrappers' own //flags still convert.
export MSYS2_ARG_CONV_EXCL="-Fo;-I;-libpath:;-out:"

# The wrapper puts $INCLUDE's directories after Pin's runtime headers: clang's own (intrinsics) must come last.
# Pin's musl defines max_align_t, which clang's stddef.h would define again for an MSVC target.
INCLUDE="$(w "$clang_inc")" "$pin/intel64/pinrt/bin/pin-clang-cl++" -c "$(w "$here/callgraph.cpp")" -Fo"$(w "$out/callgraph.obj")" \
    //std:c++17 //O2 //MD //Zi //W3 -Wno-non-c-typedef-for-linkage -Wno-microsoft-include -Wno-unicode \
    -Wno-unused-command-line-argument -D__CLANG_MAX_ALIGN_T_DEFINED -DPIN_DISABLE_CRT_REG_DEF -D_XOPEN_SOURCE=700 -D_POSIX_C_SOURCE=200809L \
    -I"$(w "$pin/source/include/pin")" -I"$(w "$pin/source/include/pin/gen")" \
    -I"$(w "$pin/intel64/pinrt/include/adaptor")" -I"$(w "$pin/extras/components/include")" \
    -I"$(w "$pin/extras/xed-intel64/include/xed")" \
    -Xclang -internal-isystem -Xclang "$(w "$clang_inc")"

"$pin/intel64/pinrt/bin/pin-ld" -dll -c++ //EXPORT:main //INCREMENTAL:NO //IGNORE:4210 //IGNORE:4049 \
    //DYNAMICBASE //NXCOMPAT //OPT:REF //DEBUG \
    -libpath:"$(w "$pin/intel64/lib")" -libpath:"$(w "$pin/extras/xed-intel64/lib")" -libpath:"$(w "$sdk_lib")" \
    "$(w "$out/callgraph.obj")" pin.lib pinrt-adaptor-static.lib xed.lib kernel32.lib \
    -out:"$(w "$out/callgraph.dll")"
echo "built $(w "$out/callgraph.dll")"
