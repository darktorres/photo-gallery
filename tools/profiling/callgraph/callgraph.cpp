// Instructions retired per function, self and inclusive, with the call graph: callgrind's output format from a Pin
// tool, for what callgrind can't run (the Windows app). One file per thread, readable by cgtab.py and KCachegrind.
//
//   pin -t callgraph.dll [-o PREFIX] [-start SYMBOL] [-stop SYMBOL] -- gallery.exe ...
//
// -start zeroes every thread's counts when any thread enters SYMBOL; -stop writes the profile and ends the process
// when any thread enters it (pg_stage_ready and the other stages in crates/trace/src/lib.rs). Without -stop the
// profile is written at exit. Outputs: PREFIX.<pid>-<thread> per thread, and PREFIX.<pid>.threads listing them.
//
// The cost model:
// - Every basic block adds its instruction count to the thread's counter. In an image Pin has debug info for (a PDB
//   next to it or where it was built), the function it belongs to is the routine Pin puts it in. Pin only finds
//   those PDBs: it doesn't search a symbol path. Elsewhere (Windows' DLLs, drivers, JIT code) Pin's routines are the
//   exports, each spanning to the next, so they aren't used: a function there is named by the address it was entered
//   at, "image+0xRVA", and symbolize.py names those from the PDBs in a symbol store afterwards.
// - A shadow stack follows call and ret. Each frame keeps the return address's stack slot, so frames that ended
//   without a ret (longjmp, exception unwinding, the kernel's user-mode callbacks returning) are dropped as soon as
//   the stack pointer is above them.
// - Entering another function without a call is a tail call when the stack pointer is at the frame's entry (the
//   frame is replaced, as callgrind does), a jump back to the function below when the frame was itself entered by a
//   jump, and otherwise a frame entered by a jump (KiUserCallbackDispatcher and the other kernel entry points).
// - An indirect jump with the stack pointer at the frame's entry is a tail call too, even within one image named by
//   address, where another function can't be told from the same one: that is how Windows' control-flow-guard
//   thunks (guard_dispatch_icall_nop: jmp rax) reach the function called, so without it each thunk's frame took in
//   whatever its DLL called through it. A jump table in a frameless function looks the same, and comes out as a
//   call to "Function+0x..." of the same function.
// - Self cost is charged to the frame on top whenever the top changes; a call's inclusive cost is the counter's
//   difference between its push and its pop, charged to the (caller, call site, callee) edge.
//
// Positions are instruction addresses (callgrind's "positions: instr"): a function's cost at its start, a call's at
// the call instruction. Each file lists the images' address ranges, and symbolize.py turns the addresses into source
// lines. The tool asks Pin for no source lines itself: each lookup is a request to Pin's symbol server (pind), and
// the gallery's start waited forever on one, whether asked while instrumenting or with the threads stopped.

#include "pin.H"

#include <cstdio>
#include <cstring>
#include <string>
#include <unordered_map>
#include <vector>

KNOB<std::string> knob_out(KNOB_MODE_WRITEONCE, "pintool", "o", "callgrind.out", "output prefix");
KNOB<std::string> knob_start(KNOB_MODE_WRITEONCE, "pintool", "start", "", "zero the counts when this is entered");
KNOB<std::string> knob_stop(KNOB_MODE_WRITEONCE, "pintool", "stop", "", "write the profile and exit when entered");

namespace {

// Fast-path ids of the frame on top, besides a function's id: PENDING right after a call (or before the first
// frame), and for code named by address, one per image (UNNAMED + the image's index), so that any block of the same
// image continues the frame and a block of another image is noticed.
constexpr UINT32 PENDING = 0xffffffff;
constexpr UINT32 UNNAMED = 0xffff0000;
constexpr UINT32 NO_SITE = 0;

struct Fn {
    std::string name;
    UINT32 image;  // index into images
    ADDRINT addr;  // the routine's start, or the address it was entered at
};

struct Image {
    std::string path;
    ADDRINT low, high;
    bool named;  // Pin has its debug info
};

// Written by instrumentation callbacks, which Pin serializes under the client lock; read when the profile is
// written, with every application thread stopped or gone.
std::vector<Fn> fns;
std::vector<ADDRINT> sites{0};  // call instructions, by site id
std::vector<std::string> images{"???"};
std::unordered_map<ADDRINT, UINT32> fn_at;  // routine start, or the block an unnamed function was entered at
std::unordered_map<ADDRINT, UINT32> site_at;
std::unordered_map<std::string, UINT32> image_id;
std::unordered_map<UINT32, bool> image_named;  // by Pin's image id
std::vector<Image> loaded;                     // every image loaded

struct Frame {
    UINT32 fn;
    UINT32 fast;    // the fast-path id: fn, or the image's UNNAMED id
    UINT32 site;    // the call instruction in the frame below; NO_SITE when entered by a jump
    ADDRINT slot;   // stack slot of the return address (the stack pointer at entry, for a jump)
    UINT64 entry;   // the thread's counter when entered
    bool jumped;    // entered without a call
};

struct EdgeKey {
    UINT32 caller, site, callee;
    bool operator==(const EdgeKey& o) const { return caller == o.caller && site == o.site && callee == o.callee; }
};

struct EdgeHash {
    size_t operator()(const EdgeKey& k) const {
        UINT64 h = (UINT64(k.caller) << 32 | k.callee) * 0x9e3779b97f4a7c15ULL;
        return size_t(h ^ (UINT64(k.site) * 0xc2b2ae3d27d4eb4fULL) ^ (h >> 29));
    }
};

struct Edge {
    UINT64 calls = 0, inclusive = 0;
};

struct Thread {
    UINT64 count = 0;  // instructions retired by this thread, in blocks entered
    UINT32 top = PENDING;
    UINT64 mark = 0;    // the counter when the top frame's self cost was last charged
    bool tail = false;  // an indirect jump at the top frame's entry: the next block replaces the frame
    std::vector<Frame> stack;
    std::vector<UINT64> self;  // by function id
    std::unordered_map<EdgeKey, Edge, EdgeHash> edges;
    THREADID tid = 0;
    NATIVE_TID os_tid = 0;
    std::string name;
    bool ended = false;
};

REG thread_reg;
PIN_LOCK threads_lock;
std::vector<Thread*> threads;  // by Pin thread id

UINT32 intern(std::vector<std::string>& names, std::unordered_map<std::string, UINT32>& ids, const std::string& s) {
    auto it = ids.find(s);
    if (it != ids.end()) return it->second;
    UINT32 id = UINT32(names.size());
    names.push_back(s);
    ids.emplace(s, id);
    return id;
}

std::string short_image(const std::string& path) {
    size_t i = path.find_last_of("\\/");
    return i == std::string::npos ? path : path.substr(i + 1);
}

// --- analysis ---------------------------------------------------------------------------------------------------

inline void charge(Thread* t, UINT64 now) {
    if (!t->stack.empty() && t->top != PENDING) {
        UINT32 fn = t->stack.back().fn;
        if (fn >= t->self.size()) t->self.resize(fn + fn / 2 + 1024, 0);
        t->self[fn] += now - t->mark;
    }
    t->mark = now;
}

inline void set_top(Thread* t) { t->top = t->stack.empty() ? PENDING : t->stack.back().fast; }

inline void pop(Thread* t, UINT64 now) {
    Frame f = t->stack.back();
    t->stack.pop_back();
    if (f.fn == PENDING || t->stack.empty()) return;  // a call that never ran, or a thread's root
    Edge& e = t->edges[EdgeKey{t->stack.back().fn, f.site, f.fn}];
    e.calls += 1;
    e.inclusive += now - f.entry;
}

// Frames whose return slot is below the stack pointer have ended.
inline void pop_ended(Thread* t, ADDRINT sp, UINT64 now) {
    while (!t->stack.empty() && t->stack.back().slot < sp) pop(t, now);
}

// Every block: count it, and leave the frame alone when its function is the top's (inlined).
ADDRINT PIN_FAST_ANALYSIS_CALL arrive(Thread* t, UINT32 fn, UINT32 n) {
    t->count += n;
    return t->top != fn;
}

// A block whose fast-path id isn't the top's. fast is the block's function, or its image's UNNAMED id, and then
// entry_fn is the function named by this block's address, for a frame entered here.
void PIN_FAST_ANALYSIS_CALL enter(Thread* t, UINT32 fast, UINT32 entry_fn, UINT32 n, ADDRINT sp) {
    UINT64 now = t->count - n;  // this block's instructions belong to where it lands
    charge(t, now);
    pop_ended(t, sp, now);
    UINT32 id = fast >= UNNAMED ? entry_fn : fast;
    bool tail = t->tail;
    t->tail = false;
    if (tail && !t->stack.empty() && sp == t->stack.back().slot && t->stack.back().fn != PENDING &&
        !(fast < UNNAMED && fast == t->stack.back().fast)) {  // a jump table within a named function isn't one
        Frame f = t->stack.back();
        pop(t, now);
        t->stack.push_back(Frame{id, fast, f.site, f.slot, now, f.jumped});
        set_top(t);
        return;
    }
    if (t->stack.empty()) {
        t->stack.push_back(Frame{id, fast, NO_SITE, sp, now, true});
    } else if (t->stack.back().fn == PENDING) {
        Frame& f = t->stack.back();
        f.fn = id;
        f.fast = fast;
    } else if (t->stack.back().fast == fast) {
        // back in the function below after the frames above ended
    } else if (t->stack.back().jumped && t->stack.size() >= 2 && t->stack[t->stack.size() - 2].fast == fast) {
        pop(t, now);
    } else if (sp == t->stack.back().slot) {
        Frame f = t->stack.back();
        pop(t, now);
        t->stack.push_back(Frame{id, fast, f.site, f.slot, now, f.jumped});
    } else {
        t->stack.push_back(Frame{id, fast, NO_SITE, sp, now, true});
    }
    set_top(t);
}

// Before a call instruction; sp is the stack pointer before it pushes the return address.
void PIN_FAST_ANALYSIS_CALL on_call(Thread* t, UINT32 site, ADDRINT sp) {
    charge(t, t->count);
    pop_ended(t, sp, t->count);
    t->stack.push_back(Frame{PENDING, PENDING, site, sp - sizeof(ADDRINT), t->count, false});
    t->top = PENDING;
}

// Before an indirect jump: at the top frame's entry, a tail call, which the next block (whatever its id) completes.
void PIN_FAST_ANALYSIS_CALL on_jump(Thread* t, ADDRINT sp) {
    if (t->stack.empty() || sp != t->stack.back().slot) return;
    charge(t, t->count);
    t->tail = true;
    t->top = PENDING;
}

// Before a ret: the frame whose return slot is at sp ends, with any above it.
void PIN_FAST_ANALYSIS_CALL on_ret(Thread* t, ADDRINT sp) {
    charge(t, t->count);
    pop_ended(t, sp + sizeof(ADDRINT), t->count);
    set_top(t);
}

// --- instrumentation ------------------------------------------------------------------------------------------

UINT32 fn_of_routine(RTN rtn) {
    ADDRINT a = RTN_Address(rtn);
    auto it = fn_at.find(a);
    if (it != fn_at.end()) return it->second;
    std::string name = RTN_Name(rtn);
    if (!name.empty() && name[0] == '?') name = PIN_UndecorateSymbolName(name, UNDECORATION_NAME_ONLY);
    IMG img = SEC_Img(RTN_Sec(rtn));
    UINT32 image = IMG_Valid(img) ? intern(images, image_id, IMG_Name(img)) : 0;
    UINT32 id = UINT32(fns.size());
    fns.push_back(Fn{name, image, a});
    fn_at.emplace(a, id);
    return id;
}

// The function named by the address it was entered at, in img (or in no image).
UINT32 fn_of_address(ADDRINT a, IMG img) {
    auto it = fn_at.find(a);
    if (it != fn_at.end()) return it->second;
    char name[64];
    UINT32 image = 0;
    if (IMG_Valid(img)) {
        image = intern(images, image_id, IMG_Name(img));
        snprintf(name, sizeof name, "%s+0x%llx", short_image(IMG_Name(img)).c_str(),
                 (unsigned long long)(a - IMG_LowAddress(img)));
    } else {
        snprintf(name, sizeof name, "0x%llx", (unsigned long long)a);
    }
    UINT32 id = UINT32(fns.size());
    fns.push_back(Fn{name, image, a});
    fn_at.emplace(a, id);
    return id;
}

UINT32 site_of(INS ins) {
    ADDRINT a = INS_Address(ins);
    auto it = site_at.find(a);
    if (it != site_at.end()) return it->second;
    UINT32 id = UINT32(sites.size());
    sites.push_back(a);
    site_at.emplace(a, id);
    return id;
}

VOID trace(TRACE trace, VOID*) {
    for (BBL bbl = TRACE_BblHead(trace); BBL_Valid(bbl); bbl = BBL_Next(bbl)) {
        ADDRINT a = BBL_Address(bbl);
        RTN rtn = INS_Rtn(BBL_InsHead(bbl));
        IMG img = IMG_FindByAddress(a);
        UINT32 fast, entry_fn = 0;
        if (RTN_Valid(rtn) && IMG_Valid(img) && image_named[IMG_Id(img)]) {
            fast = fn_of_routine(rtn);
        } else {
            entry_fn = fn_of_address(a, img);
            fast = UNNAMED + fns[entry_fn].image;
        }
        UINT32 n = BBL_NumIns(bbl);
        BBL_InsertIfCall(bbl, IPOINT_BEFORE, AFUNPTR(arrive), IARG_FAST_ANALYSIS_CALL, IARG_REG_VALUE, thread_reg,
                         IARG_UINT32, fast, IARG_UINT32, n, IARG_END);
        BBL_InsertThenCall(bbl, IPOINT_BEFORE, AFUNPTR(enter), IARG_FAST_ANALYSIS_CALL, IARG_REG_VALUE, thread_reg,
                           IARG_UINT32, fast, IARG_UINT32, entry_fn, IARG_UINT32, n, IARG_REG_VALUE, REG_STACK_PTR,
                           IARG_END);
        INS tail = BBL_InsTail(bbl);
        if (INS_IsCall(tail)) {
            INS_InsertCall(tail, IPOINT_BEFORE, AFUNPTR(on_call), IARG_FAST_ANALYSIS_CALL, IARG_REG_VALUE,
                           thread_reg, IARG_UINT32, site_of(tail), IARG_REG_VALUE, REG_STACK_PTR, IARG_END);
        } else if (INS_IsRet(tail)) {
            INS_InsertCall(tail, IPOINT_BEFORE, AFUNPTR(on_ret), IARG_FAST_ANALYSIS_CALL, IARG_REG_VALUE,
                           thread_reg, IARG_REG_VALUE, REG_STACK_PTR, IARG_END);
        } else if (INS_IsBranch(tail) && INS_IsIndirectControlFlow(tail)) {
            INS_InsertCall(tail, IPOINT_BEFORE, AFUNPTR(on_jump), IARG_FAST_ANALYSIS_CALL, IARG_REG_VALUE,
                           thread_reg, IARG_REG_VALUE, REG_STACK_PTR, IARG_END);
        }
    }
}

// --- thread names -----------------------------------------------------------------------------------------------

// SetThreadDescription(GetCurrentThread(), name), as Rust's std and Qt name their threads.
VOID on_set_name(Thread* t, ADDRINT handle, ADDRINT name) {
    if (handle != ADDRINT(-2) || name == 0) return;
    UINT16 wide[128];
    size_t got = PIN_SafeCopy(wide, reinterpret_cast<VOID*>(name), sizeof wide - sizeof wide[0]) / sizeof wide[0];
    std::string s;
    for (size_t i = 0; i < got && wide[i]; i++) s += wide[i] < 0x80 ? char(wide[i]) : '?';
    t->name = s;
}

// --- start, stop, output --------------------------------------------------------------------------------------

std::vector<Thread*> all_threads() {
    PIN_GetLock(&threads_lock, 0);
    std::vector<Thread*> v;
    for (Thread* t : threads)
        if (t) v.push_back(t);
    PIN_ReleaseLock(&threads_lock);
    return v;
}

class Names {
  public:
    // "(id) name" the first time, "(id)" after: callgrind's name compression.
    std::string operator()(UINT32 key, const std::string& name) {
        auto it = ids_.find(key);
        if (it != ids_.end()) return "(" + std::to_string(it->second) + ")";
        UINT32 id = UINT32(ids_.size()) + 1;
        ids_.emplace(key, id);
        return "(" + std::to_string(id) + ") " + name;
    }

  private:
    std::unordered_map<UINT32, UINT32> ids_;
};

// A copy of the thread's counts with the frames still open closed, as if it returned from all of them now.
Thread closed(const Thread* t) {
    Thread c = *t;
    UINT64 now = c.count;
    charge(&c, now);
    while (!c.stack.empty()) pop(&c, now);
    return c;
}

void write_thread(const Thread& c, const std::string& cmd) {
    const Thread* t = &c;
    char path[512];
    snprintf(path, sizeof path, "%s.%d-%02u", knob_out.Value().c_str(), PIN_GetPid(), unsigned(t->tid));
    FILE* f = fopen(path, "w");
    if (!f) {
        fprintf(stderr, "callgraph: can't write %s\n", path);
        return;
    }
    std::vector<std::vector<std::pair<EdgeKey, Edge>>> by_caller(fns.size());
    for (auto& kv : c.edges) by_caller[kv.first.caller].push_back(kv);
    fprintf(f, "# callgrind format\nversion: 1\ncreator: photo-gallery callgraph pintool\n");
    fprintf(f, "pid: %d\ncmd: %s\npart: 1\nthread: %u\n", PIN_GetPid(), cmd.c_str(), unsigned(t->tid));
    fprintf(f, "desc: Thread: %s (OS id %u)\n", t->name.empty() ? "-" : t->name.c_str(), unsigned(t->os_tid));
    for (const Image& im : loaded)
        fprintf(f, "# image 0x%llx 0x%llx %s\n", (unsigned long long)im.low, (unsigned long long)im.high,
                im.path.c_str());
    fprintf(f, "positions: instr\nevents: Ir\nsummary: %llu\n\n", (unsigned long long)c.count);
    Names ob, fn;
    for (UINT32 id = 0; id < fns.size(); id++) {
        UINT64 self = id < c.self.size() ? c.self[id] : 0;
        if (self == 0 && by_caller[id].empty()) continue;
        const Fn& x = fns[id];
        fprintf(f, "ob=%s\nfn=%s\n", ob(x.image, images[x.image]).c_str(), fn(id, x.name).c_str());
        fprintf(f, "0x%llx %llu\n", (unsigned long long)x.addr, (unsigned long long)self);
        for (auto& [k, e] : by_caller[id]) {
            const Fn& g = fns[k.callee];
            if (g.image != x.image) fprintf(f, "cob=%s\n", ob(g.image, images[g.image]).c_str());
            fprintf(f, "cfn=%s\n", fn(k.callee, g.name).c_str());
            // A frame entered by a jump has no call instruction: its cost sits at the caller's start.
            ADDRINT site = k.site == NO_SITE ? x.addr : sites[k.site];
            fprintf(f, "calls=%llu 0x%llx\n0x%llx %llu\n", (unsigned long long)e.calls, (unsigned long long)g.addr,
                    (unsigned long long)site, (unsigned long long)e.inclusive);
        }
        fprintf(f, "\n");
    }
    fprintf(f, "totals: %llu\n", (unsigned long long)c.count);
    fclose(f);
}

std::string command_line;

void write_all() {
    std::vector<Thread*> ts = all_threads();
    char path[512];
    snprintf(path, sizeof path, "%s.%d.threads", knob_out.Value().c_str(), PIN_GetPid());
    FILE* f = fopen(path, "w");
    UINT64 total = 0;
    for (Thread* t : ts) total += t->count;
    if (f) fprintf(f, "# thread  os-id  instructions  ended  name\n");
    for (const Thread* t : ts) {
        write_thread(closed(t), command_line);
        if (f)
            fprintf(f, "%8u %6u %13llu %6s  %s\n", unsigned(t->tid), unsigned(t->os_tid),
                    (unsigned long long)t->count, t->ended ? "yes" : "no", t->name.c_str());
    }
    if (f) {
        fprintf(f, "# total %llu in %zu threads\n", (unsigned long long)total, ts.size());
        fprintf(f, "# images: \"debug\" when Pin read their PDB, \"address\" when their functions are image+0xRVA\n");
        for (const Image& im : loaded) fprintf(f, "%s  %s\n", im.named ? "debug  " : "address", im.path.c_str());
        fclose(f);
    }
    fprintf(stderr, "callgraph: %llu instructions in %zu threads, written to %s.%d-*\n", (unsigned long long)total,
            ts.size(), knob_out.Value().c_str(), PIN_GetPid());
}

VOID on_start(THREADID tid) {
    static bool done = false;
    if (done || !PIN_StopApplicationThreads(tid)) return;
    done = true;
    for (Thread* t : all_threads()) {
        t->count = t->mark = 0;
        std::fill(t->self.begin(), t->self.end(), 0);
        t->edges.clear();
        for (Frame& fr : t->stack) fr.entry = 0;
    }
    PIN_ResumeApplicationThreads(tid);
}

VOID on_stop(THREADID tid) {
    static bool done = false;
    if (done) return;
    fprintf(stderr, "callgraph: stopping the threads\n");
    if (!PIN_StopApplicationThreads(tid)) return;
    done = true;
    fprintf(stderr, "callgraph: writing\n");
    write_all();
    PIN_ExitProcess(0);
}

VOID image_load(IMG img, VOID*) {
    bool named = IMG_hasLinesData(img);
    image_named[IMG_Id(img)] = named;
    loaded.push_back(Image{IMG_Name(img), IMG_LowAddress(img), IMG_HighAddress(img), named});
    std::string base = short_image(IMG_Name(img));
    for (auto& c : base) c = char(tolower(c));
    if (base == "kernelbase.dll") {
        RTN rtn = RTN_FindByName(img, "SetThreadDescription");
        if (RTN_Valid(rtn)) {
            RTN_Open(rtn);
            RTN_InsertCall(rtn, IPOINT_BEFORE, AFUNPTR(on_set_name), IARG_REG_VALUE, thread_reg,
                           IARG_FUNCARG_ENTRYPOINT_VALUE, 0, IARG_FUNCARG_ENTRYPOINT_VALUE, 1, IARG_END);
            RTN_Close(rtn);
        }
    }
    for (auto [knob, fun] : {std::pair{&knob_start, AFUNPTR(on_start)}, std::pair{&knob_stop, AFUNPTR(on_stop)}}) {
        if (knob->Value().empty()) continue;
        RTN rtn = RTN_FindByName(img, knob->Value().c_str());
        if (!RTN_Valid(rtn)) continue;
        fprintf(stderr, "callgraph: -%s %s in %s\n", knob == &knob_start ? "start" : "stop", knob->Value().c_str(),
                base.c_str());
        RTN_Open(rtn);
        RTN_InsertCall(rtn, IPOINT_BEFORE, fun, IARG_THREAD_ID, IARG_END);
        RTN_Close(rtn);
    }
}

VOID thread_start(THREADID tid, CONTEXT* ctxt, INT32, VOID*) {
    Thread* t = new Thread;
    t->tid = tid;
    t->os_tid = PIN_GetNativeTid();
    PIN_SetContextReg(ctxt, thread_reg, ADDRINT(t));
    PIN_GetLock(&threads_lock, tid + 1);
    if (threads.size() <= tid) threads.resize(tid + 1, nullptr);
    threads[tid] = t;
    PIN_ReleaseLock(&threads_lock);
}

VOID thread_fini(THREADID tid, const CONTEXT* ctxt, INT32, VOID*) {
    Thread* t = reinterpret_cast<Thread*>(PIN_GetContextReg(ctxt, thread_reg));
    if (!t) return;
    charge(t, t->count);
    while (!t->stack.empty()) pop(t, t->count);
    t->top = PENDING;
    t->ended = true;
}

VOID fini(INT32, VOID*) { write_all(); }

}  // namespace

int main(int argc, char* argv[]) {
    PIN_InitSymbolsAlt(DEBUG_AND_EXPORT_SYMBOLS);
    if (PIN_Init(argc, argv)) {
        fprintf(stderr, "%s\n", KNOB_BASE::StringKnobSummary().c_str());
        return 1;
    }
    for (int i = 0; i < argc; i++)
        if (std::strcmp(argv[i], "--") == 0) {
            for (int j = i + 1; j < argc; j++) command_line += std::string(j > i + 1 ? " " : "") + argv[j];
            break;
        }
    thread_reg = PIN_ClaimToolRegister();
    PIN_InitLock(&threads_lock);
    fns.reserve(1 << 16);
    TRACE_AddInstrumentFunction(trace, nullptr);
    IMG_AddInstrumentFunction(image_load, nullptr);
    PIN_AddThreadStartFunction(thread_start, nullptr);
    PIN_AddThreadFiniFunction(thread_fini, nullptr);
    PIN_AddFiniFunction(fini, nullptr);
    PIN_StartProgram();
    return 0;
}
