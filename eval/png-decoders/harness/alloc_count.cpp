#include "alloc_count.h"

#include <atomic>
#include <cstdint>
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

// The requested size of every block counted while counting was on, by pointer, so that a free subtracts the size the
// allocation added (the requested bytes on both sides, not the allocator's rounded size). The table is the C library's
// (calloc, not the counted operator new, which would recurse into here); a spinlock guards it, since operator new runs on
// any thread. Open addressing with tombstones; it doubles past half full.
struct Slot {
    void* p;
    size_t n;
};
void* const kTombstone = reinterpret_cast<void*>(uintptr_t(1));  // no allocation has this address
std::atomic_flag guard{};
Slot* slots = nullptr;
size_t capacity = 0;  // a power of two; 0 until the first block is counted
size_t occupied = 0;  // slots holding a block or a tombstone

struct Lock {
    Lock() {
        while (guard.test_and_set(std::memory_order_acquire)) {
        }
    }
    ~Lock() { guard.clear(std::memory_order_release); }
};

size_t home(const void* p) { return (reinterpret_cast<uintptr_t>(p) >> 4) & (capacity - 1); }  // blocks are 16-byte aligned

// Places a block that has no entry (the caller holds the lock, and the table has room).
void place(void* p, size_t n) {
    size_t i = home(p);
    while (slots[i].p && slots[i].p != kTombstone) i = (i + 1) & (capacity - 1);
    if (!slots[i].p) occupied++;
    slots[i] = {p, n};
}

void grow() {  // the caller holds the lock
    Slot* old = slots;
    const size_t old_capacity = capacity;
    capacity = old_capacity ? 2 * old_capacity : 1024;
    slots = static_cast<Slot*>(std::calloc(capacity, sizeof(Slot)));
    if (!slots) std::abort();
    occupied = 0;
    for (size_t i = 0; i < old_capacity; i++) {
        if (old[i].p && old[i].p != kTombstone) place(old[i].p, old[i].n);
    }
    std::free(old);
}

void track(void* p, size_t n) {
    Lock lock;
    if (2 * (occupied + 1) > capacity) grow();
    place(p, n);
}

// Removes p's entry; returns the size it was recorded with, or 0 when p was not counted.
size_t untrack(void* p) {
    Lock lock;
    if (!capacity) return 0;
    for (size_t i = home(p);; i = (i + 1) & (capacity - 1)) {
        if (!slots[i].p) return 0;
        if (slots[i].p == p) {
            slots[i].p = kTombstone;
            return slots[i].n;
        }
    }
}

// A block of n bytes was allocated at p, while counting.
void grew(void* p, size_t n) {
    calls.fetch_add(1, relaxed);
    requested.fetch_add(n, relaxed);
    track(p, n);
    live.fetch_add(int64_t(n), relaxed);
    raise_peak();
}

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
    // p's entry leaves the count here; the block that replaces it is counted again below, as a new block.
    const size_t before = p ? untrack(p) : 0;
    void* q = std::realloc(p, n);
    if (!q && n) {  // failed: p is still valid and unchanged
        if (before) track(p, before);
        return nullptr;
    }
    if (before) live.fetch_sub(int64_t(before), relaxed);
    if (q && counting.load(relaxed)) grew(q, n);
    return q;
}

void free_counted(void* p) {
    if (!p) return;
    const size_t n = untrack(p);  // 0 for a block allocated before counting was on
    if (n) live.fetch_sub(int64_t(n), relaxed);
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
