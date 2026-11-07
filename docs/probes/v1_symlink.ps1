$ErrorActionPreference = 'Stop'
# V1: a symlink root that points at an SMB admin share. Which check of decision 6 refuses it?
$testRoot = if ($env:PG_TEST_DIR) { $env:PG_TEST_DIR } else { Join-Path $PSScriptRoot '..\..\..\target\pg-test' }
$testRoot = [IO.Path]::GetFullPath($testRoot)
$link = Join-Path $testRoot 'v1-link'
# The same folder through the local administrative share (\\localhost\<drive>$\...).
$target = '\\localhost\' + $testRoot.Substring(0, 1) + '$' + $testRoot.Substring(2)
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
if (Test-Path -LiteralPath $link) { (Get-Item -LiteralPath $link -Force).Delete() }
New-Item -ItemType SymbolicLink -Path $link -Target $target | Out-Null
"link created: $link -> $target"

Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
public static class PgV1 {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern uint GetFinalPathNameByHandleW(IntPtr h, StringBuilder path, uint size, uint flags);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumePathNameW(string path, StringBuilder vol, uint volLen);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern uint GetDriveTypeW(string root);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumeInformationByHandleW(IntPtr h, StringBuilder vn, uint vnSize, out uint serial, out uint maxComp, out uint flags, StringBuilder fsn, uint fsnSize);

    const uint SHARE_ALL = 7, OPEN_EXISTING = 3, BACKUP = 0x02000000, OPEN_REPARSE = 0x00200000;

    static IntPtr Open(string p, uint flags) {
        IntPtr h = CreateFileW(p, 0, SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, flags, IntPtr.Zero);
        if (h == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error(), "open " + p);
        return h;
    }

    static string FinalPath(IntPtr h) {
        var sb = new StringBuilder(1024);
        uint n = GetFinalPathNameByHandleW(h, sb, 1024, 0);
        return n == 0 ? "error " + Marshal.GetLastWin32Error() : sb.ToString();
    }

    public static string Report(string path) {
        var sb = new StringBuilder();
        sb.AppendFormat("reparse attribute on the link itself: {0}\n", (File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0);
        var vol = new StringBuilder(261);
        bool gv = GetVolumePathNameW(path, vol, 261);
        sb.AppendFormat("step 2 GetVolumePathNameW ok={0} -> {1}\n", gv, vol);
        if (gv) sb.AppendFormat("step 3 GetDriveTypeW({0}) = {1} (3 fixed, 4 remote)\n", vol, GetDriveTypeW(vol.ToString()));
        try {
            IntPtr h = Open(path, BACKUP);
            try {
                var vn = new StringBuilder(261); var fs = new StringBuilder(261);
                uint s, m, f;
                bool ok = GetVolumeInformationByHandleW(h, vn, 261, out s, out m, out f, fs, 261);
                sb.AppendFormat("follow the link: final path {0}\n", FinalPath(h));
                sb.AppendFormat("step 6 GetVolumeInformationByHandleW ok={0} fs='{1}'\n", ok, fs);
            } finally { CloseHandle(h); }
        } catch (Win32Exception e) { sb.AppendFormat("follow the link: open failed: {0}\n", e.Message); }
        try {
            IntPtr h = Open(path, BACKUP | OPEN_REPARSE);
            try { sb.AppendFormat("no-follow handle: final path {0}\n", FinalPath(h)); }
            finally { CloseHandle(h); }
        } catch (Win32Exception e) { sb.AppendFormat("no-follow open failed: {0}\n", e.Message); }
        return sb.ToString();
    }
}
"@

[PgV1]::Report($link)
(Get-Item -LiteralPath $link -Force).Delete()
"link removed; target still exists: $(Test-Path -LiteralPath $testRoot)"
