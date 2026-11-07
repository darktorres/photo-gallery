param([string]$Dir = ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..\target\pg-test\probe'))))

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class PgProbe {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumeInformationByHandleW(IntPtr h, StringBuilder vn, uint vnSize, out uint serial, out uint maxComp, out uint flags, StringBuilder fsn, uint fsnSize);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandle(IntPtr h, out BHFI info);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandleEx(IntPtr h, int cls, byte[] buf, uint size);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumePathNameW(string path, StringBuilder vol, uint volLen);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern uint GetDriveTypeW(string root);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool MoveFileExW(string from, string to, uint flags);

    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    public struct BHFI {
        public uint attr; public long ctime; public long atime; public long wtime;
        public uint vol; public uint sh; public uint sl; public uint links; public uint ih; public uint il;
    }

    const uint FILE_SHARE_ALL = 7;
    const uint OPEN_EXISTING = 3;
    const uint BACKUP = 0x02000000;

    static IntPtr Open(string path, uint access) {
        IntPtr h = CreateFileW(path, access, FILE_SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, BACKUP, IntPtr.Zero);
        if (h == new IntPtr(-1)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "open " + path);
        return h;
    }

    public static string FileInfo(string path) {
        IntPtr h = Open(path, 0x80);
        try {
            BHFI b;
            if (!GetFileInformationByHandle(h, out b)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            byte[] buf = new byte[24];
            if (!GetFileInformationByHandleEx(h, 18, buf, 24)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            ulong vol64 = BitConverter.ToUInt64(buf, 0);
            StringBuilder id = new StringBuilder();
            for (int i = 8; i < 24; i++) id.Append(buf[i].ToString("x2"));
            ulong idx = ((ulong)b.ih << 32) | b.il;
            return string.Format("created={0:o} dwSerial=0x{1:x8} vol64=0x{2:x16} low32_eq={3} fileIndex=0x{4:x16} fileId128={5}",
                DateTime.FromFileTimeUtc(b.ctime), b.vol, vol64, ((uint)(vol64 & 0xffffffff)) == b.vol, idx, id);
        } finally { CloseHandle(h); }
    }

    public static string DirRecords(string dirPath) {
        IntPtr h = Open(dirPath, 0x1);
        try {
            byte[] buf = new byte[65536];
            if (!GetFileInformationByHandleEx(h, 10, buf, (uint)buf.Length)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            var lines = new List<string>();
            int off = 0;
            while (true) {
                int next = BitConverter.ToInt32(buf, off);
                long created = BitConverter.ToInt64(buf, off + 8);
                int nameLen = BitConverter.ToInt32(buf, off + 60);
                string name = Encoding.Unicode.GetString(buf, off + 104, nameLen);
                long fileId = BitConverter.ToInt64(buf, off + 96);
                lines.Add(string.Format("  dirlist name={0} created={1:o} FileId(dir)=0x{2:x16}", name, DateTime.FromFileTimeUtc(created), fileId));
                if (next == 0) break;
                off += next;
            }
            return string.Join("\n", lines);
        } finally { CloseHandle(h); }
    }

    public static string RootInfo(string path) {
        var sb = new StringBuilder();
        foreach (uint access in new uint[] { 0, 0x1 }) {
            IntPtr h = Open(path, access);
            try {
                var vn = new StringBuilder(261); var fsn = new StringBuilder(261);
                uint serial, maxc, flags;
                bool ok = GetVolumeInformationByHandleW(h, vn, 261, out serial, out maxc, out flags, fsn, 261);
                sb.AppendFormat("  GetVolumeInformationByHandleW(access=0x{0:x}) ok={1} fs=\"{2}\" serial=0x{3:x8} flags=0x{4:x}\n",
                    access, ok, fsn.ToString(), serial, flags);
                if (!ok) sb.AppendFormat("    error={0}\n", Marshal.GetLastWin32Error());
            } finally { CloseHandle(h); }
        }
        var vol = new StringBuilder(261);
        bool gv = GetVolumePathNameW(path, vol, 261);
        string root = vol.ToString();
        sb.AppendFormat("  GetVolumePathNameW({0}) ok={1} -> {2}\n", path, gv, root);
        sb.AppendFormat("  GetDriveTypeW({0}) = {1} (2 removable, 3 fixed, 4 remote, 5 cdrom, 6 ramdisk)\n", root, GetDriveTypeW(root));
        return sb.ToString();
    }

    public static bool MoveOver(string from, string to) { return MoveFileExW(from, to, 1); }
}
"@

$ErrorActionPreference = 'Stop'
function Info([string]$p) { [PgProbe]::FileInfo($p) }

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
Get-ChildItem -LiteralPath $Dir -Force | Remove-Item -Force
$a = Join-Path $Dir 'a.txt'
$b = Join-Path $Dir 'b.tmp'

Write-Output '== 1. Volume and file identity on the test folder'
Write-Output ([PgProbe]::RootInfo($Dir))

Write-Output '== 2. Create a.txt, then in-place overwrite (same file expected)'
[IO.File]::WriteAllText($a, 'one'); Start-Sleep -Milliseconds 300
$i0 = Info $a; Write-Output "A0 $i0"
[IO.File]::WriteAllText($a, 'two-two'); Start-Sleep -Milliseconds 300
$i1 = Info $a; Write-Output "A1 in-place $i1"
Write-Output ("  same created: {0}  same fileId: {1}" -f (($i0 -replace ' dwSerial.*','') -eq ($i1 -replace ' dwSerial.*','')), (($i0 -replace '.*fileId128=','') -eq ($i1 -replace '.*fileId128=','')))

Write-Output '== 3. Save-by-rename (write temp, MoveFileEx over a.txt), the common editor pattern'
[IO.File]::WriteAllText($b, 'three'); Start-Sleep -Milliseconds 300
$bId = Info $b; Write-Output "B (temp) $bId"
[PgProbe]::MoveOver($b, $a) | Out-Null
$i2 = Info $a; Write-Output "A2 after rename-over $i2"
Write-Output ("  fileId changed: {0}  created changed: {1}" -f (($i2 -replace '.*fileId128=','') -ne ($i1 -replace '.*fileId128=','')), (($i2 -replace ' dwSerial.*','') -ne ($i1 -replace ' dwSerial.*','')))

Write-Output '== 4. File.Replace over a.txt (the other common save API)'
$tmpReplace = Join-Path $Dir 'c.tmp'
[IO.File]::WriteAllText($tmpReplace, 'four'); Start-Sleep -Milliseconds 300
$bak = Join-Path $Dir 'bak.tmp'; if (Test-Path -LiteralPath $bak) { Remove-Item -LiteralPath $bak -Force }
[IO.File]::Replace($tmpReplace, $a, $bak)
$i3 = Info $a; Write-Output "A3 after File.Replace $i3"
Write-Output ("  fileId changed vs step 3: {0}" -f (($i3 -replace '.*fileId128=','') -ne ($i2 -replace '.*fileId128=','')))

Write-Output '== 5. Delete and recreate the same name, several cycles (record reuse)'
$prev = $i3 -replace '.*fileId128=',''
for ($n = 1; $n -le 5; $n++) {
    Remove-Item -LiteralPath $a -Force; Start-Sleep -Milliseconds 100
    [IO.File]::WriteAllText($a, "cycle$n"); Start-Sleep -Milliseconds 100
    $cur = Info $a
    $id = $cur -replace '.*fileId128=',''
    Write-Output "  cycle $n fileId=$id reused-previous=$($id -eq $prev)  $($cur -replace ' fileId128=.*','')"
    $prev = $id
}

Write-Output '== 6. Rename keeps identity'
$r = Join-Path $Dir 'renamed.txt'
$before = Info $a
Rename-Item -LiteralPath $a -NewName 'renamed.txt'
$after = Info $r
Write-Output "  before $before"
Write-Output "  after  $after"
Write-Output ("  same fileId after rename: {0}" -f (($before -replace '.*fileId128=','') -eq ($after -replace '.*fileId128=','')))

Write-Output '== 7. Directory listing (FileIdBothDirectoryInfo) gives the same identity and creation time'
Write-Output ([PgProbe]::DirRecords($Dir))

Write-Output '== 8. Z: and C: roots (read-only)'
foreach ($root in @('C:\', 'Z:\')) {
    if (Test-Path -LiteralPath $root) { Write-Output "root $root"; Write-Output ([PgProbe]::RootInfo($root)) }
    else { Write-Output "root $root not present" }
}

Get-ChildItem -LiteralPath $Dir -Force | Remove-Item -Force
Write-Output '== done'
