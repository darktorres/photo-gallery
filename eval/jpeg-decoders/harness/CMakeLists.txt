# The decoder harness (harness.cpp). Every decoder is optional: give its paths and it is compiled in.
#   libjpeg-turbo (required, the reference): TURBO_INCLUDE (turbojpeg.h), TURBO_LIB
#   parajpeg:  PARAJPEG_SRC, PARAJPEG_LIB (built with PARAJPEG_WITH_CUDA=ON for its CUDA backend: also WITH_CUDA)
#   jpegli:    JPEGLI_SRC, JPEGLI_BUILD (its CMake build directory)
#   Wuffs:     WUFFS_SRC (the repository)
#   GPU:       WITH_CUDA=ON for nvJPEG (CUDA toolkit); JPEGGPU_SRC, JPEGGPU_BUILD; GPUJPEG_SRC, GPUJPEG_BUILD
# eval/jpeg-decoders/win-build.cmd and wsl-build.sh configure it.
cmake_minimum_required(VERSION 3.20)
project(jpeg_harness CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

option(WITH_CUDA "nvJPEG, and CUDA for parajpeg/jpeggpu/GPUJPEG" OFF)

add_executable(harness harness.cpp dec_turbo.cpp dec_parajpeg.cpp dec_jpegli.cpp dec_wuffs.cpp dec_gpu.cpp)
set_source_files_properties(dec_turbo.cpp PROPERTIES INCLUDE_DIRECTORIES "${TURBO_INCLUDE}")
target_link_libraries(harness PRIVATE "${TURBO_LIB}")
if(NOT WIN32)
  target_link_libraries(harness PRIVATE pthread)
endif()

if(PARAJPEG_SRC)
  set_source_files_properties(dec_parajpeg.cpp PROPERTIES INCLUDE_DIRECTORIES "${PARAJPEG_SRC}/include"
                              COMPILE_DEFINITIONS HAVE_PARAJPEG)
  target_link_libraries(harness PRIVATE "${PARAJPEG_LIB}")
  if(WIN32)  # built with clang-cl: __builtin_cpu_supports needs clang's builtins runtime
    file(GLOB CLANG_RT "C:/Program Files/LLVM/lib/clang/*/lib/windows/clang_rt.builtins-x86_64.lib")
    target_link_libraries(harness PRIVATE ${CLANG_RT})
  endif()
endif()

if(JPEGLI_SRC)
  set_source_files_properties(dec_jpegli.cpp PROPERTIES
    INCLUDE_DIRECTORIES "${JPEGLI_SRC};${JPEGLI_BUILD}/lib/include/jpegli" COMPILE_DEFINITIONS HAVE_JPEGLI)
  target_link_libraries(harness PRIVATE "${JPEGLI_BUILD}/lib/libjpegli-static.a"
                        "${JPEGLI_BUILD}/third_party/highway/libhwy.a")
endif()

if(WUFFS_SRC)
  set_source_files_properties(dec_wuffs.cpp PROPERTIES
    COMPILE_DEFINITIONS "HAVE_WUFFS;WUFFS_SOURCE=\"${WUFFS_SRC}/release/c/wuffs-unsupported-snapshot.c\"")
endif()

if(WITH_CUDA)
  find_package(CUDAToolkit REQUIRED)
  set(GPU_DEFS HAVE_CUDA HAVE_NVJPEG)
  set(GPU_INCS "")
  target_link_libraries(harness PRIVATE CUDA::cudart CUDA::nvjpeg)
  if(JPEGGPU_SRC)
    list(APPEND GPU_DEFS HAVE_JPEGGPU)
    list(APPEND GPU_INCS "${JPEGGPU_SRC}/include")
    target_link_libraries(harness PRIVATE "${JPEGGPU_BUILD}/jpeggpu${CMAKE_STATIC_LIBRARY_SUFFIX}")
  endif()
  if(GPUJPEG_SRC)
    list(APPEND GPU_DEFS HAVE_GPUJPEG)
    list(APPEND GPU_INCS "${GPUJPEG_SRC}" "${GPUJPEG_BUILD}")
    target_link_libraries(harness PRIVATE "${GPUJPEG_BUILD}/gpujpeg${CMAKE_IMPORT_LIBRARY_SUFFIX}")
  endif()
  set_source_files_properties(dec_gpu.cpp PROPERTIES COMPILE_DEFINITIONS "${GPU_DEFS}"
                              INCLUDE_DIRECTORIES "${GPU_INCS}")
endif()
