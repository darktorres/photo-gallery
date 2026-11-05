param(
    [string[]]$ScanRoots = @('D:\GitHub', 'C:\Users\Torres', 'C:\Windows\System32'),
    [int]$MaxEntries = 400000,
    [string]$UncFolder = '\\localhost\D$\GitHub\photo-gallery\target'
)

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class PgProbe2 {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumeInformationByHandleW(IntPtr h, StringBuilder vn, uint vnSize, out uint serial, out uint maxComp, out uint flags, StringBuilder fsn, uint fsnSize);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandleEx(IntPtr h, int cls, byte[] buf, uint size);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumePathNameW(string path, StringBuilder vol, uint volLen);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern uint GetDriveTypeW(string root);

    const uint SHARE_ALL = 7, OPEN_EXISTING = 3, BACKUP = 0x02000000;

    static IntPtr Open(string path, uint access) {
        IntPtr h = CreateFileW(path, access, SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, BACKUP, IntPtr.Zero);
        if (h == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error(), "open " + path);
        return h;
    }

    public static string UncInfo(string path) {
        var sb = new StringBuilder();
        var vol = new StringBuilder(261);
        bool gv = GetVolumePathNameW(path, vol, 261);
        string root = vol.ToString();
        sb.AppendFormat("GetVolumePathNameW({0}) ok={1} -> '{2}' (err {3})\n", path, gv, root, gv ? 0 : Marshal.GetLastWin32Error());
        if (gv) sb.AppendFormat("GetDriveTypeW('{0}') = {1} (3 fixed, 4 remote)\n", root, GetDriveTypeW(root));
        try {
            IntPtr h = Open(path, 0);
            try {
                var vn = new StringBuilder(261); var fsn = new StringBuilder(261);
                uint serial, maxc, flags;
                bool ok = GetVolumeInformationByHandleW(h, vn, 261, out serial, out maxc, out flags, fsn, 261);
                sb.AppendFormat("GetVolumeInformationByHandleW on folder handle: ok={0} fs='{1}' serial=0x{2:x8} flags=0x{3:x}\n", ok, fsn, serial, flags);
                if (!ok) sb.AppendFormat("  error {0}\n", Marshal.GetLastWin32Error());
            } finally { CloseHandle(h); }
        } catch (Win32Exception e) {
            sb.AppendFormat("open folder failed: {0} (native {1})\n", e.Message, e.NativeErrorCode);
        }
        return sb.ToString();
    }

    // Walks the trees, listing with FileIdBothDirectoryInfo. Reports non-dot entries whose FileId is 0.
    public static string ZeroScan(string[] roots, int maxEntries) {
        var sb = new StringBuilder();
        var stack = new Stack<string>();
        foreach (var r in roots) stack.Push(r);
        int entries = 0, zeros = 0, dirs = 0, reparseDirs = 0, failures = 0;
        var zeroSamples = new List<string>();
        var failSamples = new List<string>();
        while (stack.Count > 0 && entries < maxEntries) {
            string dir = stack.Pop();
            IntPtr h;
            try { h = Open(dir, 0x1); }
            catch (Win32Exception) { failures++; if (failSamples.Count < 5) failSamples.Add(dir); continue; }
            try {
                dirs++;
                byte[] buf = new byte[65536];
                while (true) {
                    if (!GetFileInformationByHandleEx(h, 10, buf, (uint)buf.Length)) break;
                    int off = 0;
                    while (true) {
                        int next = BitConverter.ToInt32(buf, off);
                        uint attrs = BitConverter.ToUInt32(buf, off + 56);
                        int nameLen = BitConverter.ToInt32(buf, off + 60);
                        string name = Encoding.Unicode.GetString(buf, off + 104, nameLen);
                        long fileId = BitConverter.ToInt64(buf, off + 96);
                        bool isDot = name == "." || name == "..";
                        if (!isDot) {
                            entries++;
                            if (fileId == 0) {
                                zeros++;
                                if (zeroSamples.Count < 20) zeroSamples.Add(dir + "\\" + name + "  attrs=0x" + attrs.ToString("x"));
                            }
                            if ((attrs & 0x10) != 0 && (attrs & 0x400) == 0) stack.Push(dir + "\\" + name);
                            if ((attrs & 0x10) != 0 && (attrs & 0x400) != 0) reparseDirs++;
                        }
                        if (next == 0) break;
                        off += next;
                    }
                }
            } finally { CloseHandle(h); }
            if (entries >= maxEntries) break;
        }
        sb.AppendFormat("entries scanned={0} folders listed={1} non-dot FileId==0: {2} reparse folders seen={3} folder open failures={4} stack left={5}\n",
            entries, dirs, zeros, reparseDirs, failures, stack.Count);
        foreach (var s in zeroSamples) sb.AppendLine("  zero: " + s);
        foreach (var s in failSamples) sb.AppendLine("  open failed: " + s);
        return sb.ToString();
    }
}
"@

$ErrorActionPreference = 'Continue'
Write-Output '== A. UNC path (SMB admin share on this machine)'
try { Write-Output ([PgProbe2]::UncInfo($UncFolder)) } catch { Write-Output "UNC probe threw: $($_.Exception.Message)" }

Write-Output '== B. Zero file IDs on local NTFS (non-dot entries only)'
Write-Output ([PgProbe2]::ZeroScan($ScanRoots, $MaxEntries))
