$ErrorActionPreference = 'Stop'
# V1 (no elevation): a junction on D: whose target is a folder on C:. Shows what the path-based checks report
# for a reparse root whose real volume differs from the path's volume. Junction removal deletes the link only.
$testRoot = 'D:\GitHub\photo-gallery\target\pg-test'
$link = Join-Path $testRoot 'v1-junction'
$target = 'C:\Users\Torres'
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
if (Test-Path -LiteralPath $link) { [IO.Directory]::Delete($link, $false) }
cmd /c "mklink /J `"$link`" `"$target`"" | Out-Null
"junction: $link -> $target"

Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
public static class PgV1b {
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
        IntPtr h = Open(path, BACKUP);
        try {
            var vn = new StringBuilder(261); var fs = new StringBuilder(261);
            uint s, m, f;
            bool ok = GetVolumeInformationByHandleW(h, vn, 261, out s, out m, out f, fs, 261);
            sb.AppendFormat("follow: final path {0}\n", FinalPath(h));
            sb.AppendFormat("step 6 GetVolumeInformationByHandleW ok={0} fs='{1}'\n", ok, fs);
        } finally { CloseHandle(h); }
        h = Open(path, BACKUP | OPEN_REPARSE);
        try { sb.AppendFormat("no-follow handle: final path {0}\n", FinalPath(h)); }
        finally { CloseHandle(h); }
        return sb.ToString();
    }
}
"@

[PgV1b]::Report($link)
[IO.Directory]::Delete($link, $false)
"junction removed; C: target still exists: $(Test-Path -LiteralPath $target)"
