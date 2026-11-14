#include "alloc_count.h"

#include <malloc.h>

#include <atomic>
#include <cstdlib>
#include <new>

namespace heap {
namespace {

constexpr auto relaxed = std::memory_order_relaxed;

std::atomic<bool> counting{false};
std::atomic<uint64_t> calls{0}, requested{0};
std::atomic<int64_t> live{0}, peak{0};

void raise_peak() {
    if (live.load(relaxed) > peak.load(relaxed)) peak.store(live.load(relaxed), relaxed);
}

// A block of n bytes was allocated at p.
void grew(void* p, size_t n) {
    calls.fetch_add(1, relaxed);
    requested.fetch_add(n, relaxed);
    live.fetch_add(int64_t(_msize(p)), relaxed);
    raise_peak();
}

void shrank(void* p) { live.fetch_sub(int64_t(_msize(p)), relaxed); }

// Used by operator new and delete: a non-null block or a throw.
void* checked_new(size_t n) {
    void* p = malloc_counted(n ? n : 1);
    if (!p) throw std::bad_alloc();
    return p;
}

}  // namespace

void enable(bool on) { counting.store(on, relaxed); }

Totals totals() {
    return {calls.load(relaxed), requested.load(relaxed), live.load(relaxed), peak.load(relaxed)};
}

void reset() {
    calls.store(0, relaxed);
    requested.store(0, relaxed);
    peak.store(live.load(relaxed), relaxed);
}

void* malloc_counted(size_t n) {
    void* p = std::malloc(n);
    if (p && counting.load(relaxed)) grew(p, n);
    return p;
}

void* calloc_counted(size_t count, size_t size) {
    void* p = std::calloc(count, size);
    if (p && counting.load(relaxed)) grew(p, count * size);
    return p;
}

void* realloc_counted(void* p, size_t n) {
    if (!counting.load(relaxed)) return std::realloc(p, n);
    const int64_t before = p ? int64_t(_msize(p)) : 0;
    void* q = std::realloc(p, n);
    if (!q) return nullptr;  // p is still valid and nothing changed
    calls.fetch_add(1, relaxed);
    requested.fetch_add(n, relaxed);
    live.fetch_add(int64_t(_msize(q)) - before, relaxed);
    raise_peak();
    return q;
}

void free_counted(void* p) {
    if (!p) return;
    if (counting.load(relaxed)) shrank(p);
    std::free(p);
}

}  // namespace heap

// The global allocation functions: every operator new and delete of the process goes through heap::, so the counts
// include the C++ library's and the harness's own allocations.
void* operator new(size_t n) { return heap::checked_new(n); }
void* operator new[](size_t n) { return heap::checked_new(n); }
void* operator new(size_t n, const std::nothrow_t&) noexcept { return heap::malloc_counted(n ? n : 1); }
void* operator new[](size_t n, const std::nothrow_t&) noexcept { return heap::malloc_counted(n ? n : 1); }
void operator delete(void* p) noexcept { heap::free_counted(p); }
void operator delete[](void* p) noexcept { heap::free_counted(p); }
void operator delete(void* p, size_t) noexcept { heap::free_counted(p); }
void operator delete[](void* p, size_t) noexcept { heap::free_counted(p); }
void operator delete(void* p, const std::nothrow_t&) noexcept { heap::free_counted(p); }
void operator delete[](void* p, const std::nothrow_t&) noexcept { heap::free_counted(p); }
