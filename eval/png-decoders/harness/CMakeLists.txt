# The C side's harness (harness.cpp): libpng, libspng and Wuffs, as static libraries from win_build.py.
#   LIBPNG_INCLUDE, LIBPNG_LIB   (libpng's install: include/libpng16, lib/libpng16_static.lib)
#   SPNG_INCLUDE, SPNG_LIB       (libspng's install, a static build: SPNG_STATIC is defined for it below)
#   ZLIB_LIB, ZLIB_INCLUDE       (zlib's static libz.lib and its zlib.h; libpng and libspng both use it, and `inflate` calls it)
#   WUFFS_SRC                    (the Wuffs repository; release/c/wuffs-unsupported-snapshot.c is compiled in)
# Built with MSVC (CMAKE_CXX_COMPILER=cl) and Release, which the measurements need.
cmake_minimum_required(VERSION 3.20)
project(png_c_harness CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()
if(NOT CMAKE_BUILD_TYPE STREQUAL "Release")
  message(FATAL_ERROR "png-c-harness is measured as Release (CMAKE_BUILD_TYPE is ${CMAKE_BUILD_TYPE})")
endif()

add_executable(png-c-harness
  harness.cpp
  png_info.cpp
  alloc_count.cpp
  dec_libpng.cpp
  dec_libspng.cpp
  dec_wuffs.cpp)

set_source_files_properties(harness.cpp PROPERTIES INCLUDE_DIRECTORIES "${ZLIB_INCLUDE}")
set_source_files_properties(dec_libpng.cpp PROPERTIES INCLUDE_DIRECTORIES "${LIBPNG_INCLUDE}")
set_source_files_properties(dec_libspng.cpp PROPERTIES INCLUDE_DIRECTORIES "${SPNG_INCLUDE}")
set_source_files_properties(dec_wuffs.cpp PROPERTIES
  COMPILE_DEFINITIONS "WUFFS_SOURCE=\"${WUFFS_SRC}/release/c/wuffs-unsupported-snapshot.c\"")
target_compile_definitions(png-c-harness PRIVATE SPNG_STATIC)
target_link_libraries(png-c-harness PRIVATE "${LIBPNG_LIB}" "${SPNG_LIB}" "${ZLIB_LIB}")
