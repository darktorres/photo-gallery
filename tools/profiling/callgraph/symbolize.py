#!/usr/bin/env python3
"""Turns callgraph.dll's raw profiles into ones with names and source lines, in place, from PDBs.

callgraph.dll writes instruction addresses as positions ("positions: instr"), with the address range of every image
in the file's header, and names a function by address ("ntdll.dll+0x1a2b0") where Pin had no PDB for its image:
Pin reads a PDB only from beside its DLL or from where it was built, and the tool asks Pin for no source lines (each
lookup is a request to Pin's symbol server, which hung the gallery). This does both with dbghelp, through a symbol
path: the directories given with --path, then Microsoft's symbol server, cached in D:\\symbols.

- Positions become source lines ("positions: line"), with fl=, fi= and cfi= naming the files, so the profile reads
  as callgrind's own. An address without line information (Windows' public PDBs have none) becomes line 0 of "???".
- A function named by address gets its symbol's name, or "Name+0x40" when it was entered somewhere other than its
  start (a jump into its middle). A DLL without a PDB (the GPU driver) gets dbghelp's nearest export and offset.
- Rust's mangled names (the v0 scheme: "_RNvNtCs..._7pg_core6browse10list_files", as the PDB has some of them) are
  demangled with LLVM's llvm-cxxfilt ("pg_core::browse::list_files"), without LTO's ".llvm.NNNN" suffix.

    symbolize.py [--path DIR]... [--cache DIR] FILE...

Needs the Windows SDK's x64 debugging tools (dbghelp.dll with symsrv.dll beside it, for the symbol server), and LLVM
for the Rust names.
"""

import argparse
import bisect
import ctypes
import os
import re
import subprocess
import sys
from ctypes import wintypes
from typing import ClassVar

DEBUGGERS = r"C:\Program Files\Windows Kits\10\Debuggers\x64"
CXXFILT = r"C:\Program Files\LLVM\bin\llvm-cxxfilt.exe"
MANGLED = re.compile(r"^(c?fn=\(\d+\) )(_R[0-9A-Za-z_$.]+)$")
LTO_SUFFIX = re.compile(r"\.llvm\.\d+$")
SERVER = "https://msdl.microsoft.com/download/symbols"
COMPRESSED = re.compile(r"^(\(\d+\))(?: (.+))?$")
BY_ADDRESS = re.compile(r"^(?P<image>[^+\s]+)\+0x(?P<rva>[0-9a-f]+)$")
IMAGE = re.compile(r"^# image 0x([0-9a-f]+) 0x([0-9a-f]+) (.+)$")

SYMOPT_UNDNAME = 0x2
SYMOPT_LOAD_LINES = 0x10
SYMOPT_FAIL_CRITICAL_ERRORS = 0x200
SYMOPT_NO_PROMPTS = 0x80000


class SymbolInfo(ctypes.Structure):
    _fields_: ClassVar = [
        ("SizeOfStruct", wintypes.ULONG),
        ("TypeIndex", wintypes.ULONG),
        ("Reserved", ctypes.c_uint64 * 2),
        ("Index", wintypes.ULONG),
        ("Size", wintypes.ULONG),
        ("ModBase", ctypes.c_uint64),
        ("Flags", wintypes.ULONG),
        ("Value", ctypes.c_uint64),
        ("Address", ctypes.c_uint64),
        ("Register", wintypes.ULONG),
        ("Scope", wintypes.ULONG),
        ("Tag", wintypes.ULONG),
        ("NameLen", wintypes.ULONG),
        ("MaxNameLen", wintypes.ULONG),
        ("Name", wintypes.WCHAR * 1024),
    ]


class LineInfo(ctypes.Structure):  # IMAGEHLP_LINEW64
    _fields_: ClassVar = [
        ("SizeOfStruct", wintypes.DWORD),
        ("Key", ctypes.c_void_p),
        ("LineNumber", wintypes.DWORD),
        ("FileName", ctypes.c_wchar_p),
        ("Address", ctypes.c_uint64),
    ]


class Symbols:
    """dbghelp over the images of one profiled process, each loaded at the address it had there."""

    def __init__(self, search_path):
        # dbghelp loads symsrv.dll by name, when it first needs the server.
        ctypes.windll.kernel32.SetDllDirectoryW(DEBUGGERS)
        d = self.dbghelp = ctypes.WinDLL(os.path.join(DEBUGGERS, "dbghelp.dll"))
        d.SymInitializeW.argtypes = [wintypes.HANDLE, wintypes.LPCWSTR, wintypes.BOOL]
        d.SymLoadModuleExW.restype = ctypes.c_uint64
        d.SymLoadModuleExW.argtypes = [
            wintypes.HANDLE, wintypes.HANDLE, wintypes.LPCWSTR, wintypes.LPCWSTR,
            ctypes.c_uint64, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD,
        ]  # fmt: skip
        d.SymFromAddrW.argtypes = [
            wintypes.HANDLE, ctypes.c_uint64, ctypes.POINTER(ctypes.c_uint64), ctypes.POINTER(SymbolInfo),
        ]  # fmt: skip
        d.SymGetLineFromAddrW64.argtypes = [
            wintypes.HANDLE, ctypes.c_uint64, ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(LineInfo),
        ]  # fmt: skip
        # Not a process: dbghelp only wants a unique non-null handle when it isn't asked to read a process.
        # Not SYMOPT_DEFERRED_LOADS: SymFromAddr doesn't load a deferred module's PDB here. Only the images a profile
        # has addresses in are loaded, each once.
        self.handle = wintypes.HANDLE(0x5EB0)
        d.SymSetOptions(SYMOPT_UNDNAME | SYMOPT_LOAD_LINES | SYMOPT_FAIL_CRITICAL_ERRORS | SYMOPT_NO_PROMPTS)
        if not d.SymInitializeW(self.handle, search_path, False):
            sys.exit(f"SymInitialize failed: {ctypes.GetLastError()}")
        self.images = []  # (low, high, path), sorted by low
        self.lows = []
        self.loaded = {}  # path -> loaded
        self.names = {}
        self.lines = {}

    def add_image(self, low, high, path):
        if (low, high, path) in self.images:
            return
        i = bisect.bisect(self.lows, low)
        self.images.insert(i, (low, high, path))
        self.lows.insert(i, low)

    def _load(self, low, high, path):
        if path not in self.loaded:
            self.loaded[path] = bool(self.dbghelp.SymLoadModuleExW(self.handle, None, path, None, low, high - low,
                                                                   None, 0))  # fmt: skip
        return self.loaded[path]

    def _image_at(self, addr):
        i = bisect.bisect(self.lows, addr) - 1
        if i >= 0:
            low, high, path = self.images[i]
            if addr < high and self._load(low, high, path):
                return True
        return False

    def base_of(self, path):
        for low, high, p in self.images:
            if p.lower() == path.lower() and self._load(low, high, p):
                return low
        return None

    def name(self, addr):
        if addr not in self.names:
            self.names[addr] = None
            if self._image_at(addr):
                info = SymbolInfo()
                info.SizeOfStruct = 88  # sizeof(SYMBOL_INFOW) on x64, with its one-character Name
                info.MaxNameLen = 1024
                disp = ctypes.c_uint64()
                if self.dbghelp.SymFromAddrW(self.handle, addr, ctypes.byref(disp), ctypes.byref(info)):
                    self.names[addr] = info.Name if disp.value == 0 else f"{info.Name}+0x{disp.value:x}"
        return self.names[addr]

    def line(self, addr):
        """(file, line) of an instruction, ("???", 0) when unknown."""
        if addr not in self.lines:
            self.lines[addr] = ("???", 0)
            if self._image_at(addr):
                info = LineInfo()
                info.SizeOfStruct = ctypes.sizeof(LineInfo)
                disp = wintypes.DWORD()
                if self.dbghelp.SymGetLineFromAddrW64(self.handle, addr, ctypes.byref(disp), ctypes.byref(info)):
                    self.lines[addr] = (info.FileName, info.LineNumber)
        return self.lines[addr]


class Demangler:
    """Rust v0 names, through one llvm-cxxfilt run per batch."""

    def __init__(self):
        self.cache = {}
        self.missing = not os.path.exists(CXXFILT)

    def demangle(self, names):
        todo = sorted({LTO_SUFFIX.sub("", n) for n in names} - self.cache.keys())
        if todo and not self.missing:
            out = subprocess.run([CXXFILT], input="\n".join(todo), capture_output=True, text=True, check=True)
            self.cache.update(zip(todo, out.stdout.splitlines(), strict=True))
        return {n: self.cache.get(LTO_SUFFIX.sub("", n), n) for n in names}


class Files:
    """callgrind's compressed file names: "(id) path" the first time, "(id)" after."""

    def __init__(self):
        self.ids = {}

    def __call__(self, path):
        if path in self.ids:
            return self.ids[path]
        self.ids[path] = f"({len(self.ids) + 1})"
        return f"{self.ids[path]} {path}"


def address(token):
    return int(token, 16) if token.startswith("0x") else int(token)


def symbolize(path, symbols, demangler):
    """Rewrites PATH. Returns (functions named, functions left by address, positions without a line)."""
    with open(path, encoding="utf-8", errors="replace") as f:
        lines = f.read().split("\n")
    if "positions: instr" not in lines:
        return None  # already done, or not callgraph.dll's
    for line in lines:
        m = IMAGE.match(line)
        if m:
            symbols.add_image(int(m.group(1), 16), int(m.group(2), 16), m.group(3))
        elif line.startswith("positions:"):
            break

    objects = {}  # compressed ob id -> path, for ob= and cob=
    ob = None
    files = Files()
    named = left = unknown = 0
    out = []
    held = []  # the lines of a fn= or a call, until the cost line that gives their positions
    cur_file = None
    after_calls = False

    def object_of(value):
        m = COMPRESSED.match(value)
        if m and m.group(2):
            objects[m.group(1)] = m.group(2)
        return objects.get(m.group(1), value) if m else value

    def rename(key, value, image):
        nonlocal named, left
        m = COMPRESSED.match(value)
        if not (m and m.group(2) and image):
            return f"{key}={value}"
        a = BY_ADDRESS.match(m.group(2))
        if not (a and os.path.basename(image).lower() == a.group("image").lower()):
            return f"{key}={value}"
        base = symbols.base_of(image)
        name = symbols.name(base + int(a.group("rva"), 16)) if base is not None else None
        if not name:
            left += 1
            return f"{key}={value}"
        named += 1
        return f"{key}={m.group(1)} {name}"

    def where(addr):
        nonlocal unknown
        file, line = symbols.line(addr)
        if line == 0:
            unknown += 1
        return file, line

    cob = None
    for line in lines:
        if line.startswith("positions:"):
            out.append("positions: line")
            continue
        key, sep, value = line.partition("=")
        if sep and key == "ob":
            ob = object_of(value)
            out.append(line)
        elif sep and key == "cob":
            cob = object_of(value)
            held.append(line)
        elif sep and key == "fn":
            held = [rename("fn", value, ob)]
        elif sep and key == "cfn":
            held.append(rename("cfn", value, cob or ob))
            cob = None
        elif sep and key == "calls":
            count, target = value.split()
            file, ln = where(address(target))
            # cfi= before the cfn= it belongs to: held is [cob=]? cfn=
            held.insert(len(held) - 1, f"cfi={files(file)}")
            held.append(f"calls={count} {ln}")
            after_calls = True
        elif line[:1].isdigit():
            pos, _, costs = line.partition(" ")
            file, ln = where(address(pos))
            if after_calls:
                if file != cur_file:
                    out.append(f"fi={files(file)}")
                    cur_file = file
                out.extend(held)
                after_calls = False
            else:  # the function's own cost, at its start: its file
                out.append(f"fl={files(file)}")
                out.extend(held)
                cur_file = file
            held = []
            out.append(f"{ln} {costs}")
        else:
            out.append(line)
    mangled = [m for m in map(MANGLED.match, out) if m]
    plain = demangler.demangle([m.group(2) for m in mangled])
    for i, line in enumerate(out):
        m = MANGLED.match(line)
        if m:
            out[i] = m.group(1) + plain[m.group(2)]
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))
    return named, left, unknown


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+")
    ap.add_argument("--path", action="append", default=[], help="another directory of PDBs (repeatable)")
    ap.add_argument("--cache", default=r"D:\symbols", help="where the symbol server's PDBs are kept")
    args = ap.parse_args()
    symbols = Symbols(";".join([*args.path, f"srv*{args.cache}*{SERVER}"]))
    demangler = Demangler()
    if demangler.missing:
        print(f"no {CXXFILT}: Rust names stay mangled")
    for p in args.files:
        r = symbolize(p, symbols, demangler)
        if r is None:
            print(f"{p}: skipped (not a raw callgraph.dll profile)")
        else:
            print(f"{p}: {r[0]} functions named, {r[1]} left by address, {r[2]} positions without a line")


if __name__ == "__main__":
    main()
