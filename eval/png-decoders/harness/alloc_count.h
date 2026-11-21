// Heap-call counting for the allocs command. It covers the process's global operator new and delete, and the C
// allocators the libraries call through: libpng's and libspng's user allocators (their zlib state goes through them
// too) and Wuffs' calloc and free (dec_wuffs.cpp routes them here). Counting is off unless heap::enable(true), so the
// time and compare commands run with each call's one flag test and nothing else.
#pragma once
#include <cstddef>
#include <cstdint>

namespace heap {

void enable(bool on);

// calls: allocation and reallocation calls since reset(); requested: the bytes they asked for; live: the bytes
// allocated while counting and not yet freed (each block's requested size, as recorded at its allocation); peak: the
// largest live since reset().
struct Totals {
    uint64_t calls;
    uint64_t requested;
    int64_t live;
    int64_t peak;
};
Totals totals();
void reset();  // zeros calls and requested, and sets peak to the live bytes now

void* malloc_counted(size_t n);
void* calloc_counted(size_t count, size_t size);
void* realloc_counted(void* p, size_t n);
void free_counted(void* p);

}  // namespace heap
