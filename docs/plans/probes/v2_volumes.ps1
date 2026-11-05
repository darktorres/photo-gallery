$ErrorActionPreference = 'Stop'
# V2: every volume on this machine, its 32-bit serial (GetVolumeInformation) and 64-bit FileIdInfo serial,
# and any two volumes whose 64-bit serials share the low 32 bits.

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class PgV2 {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr FindFirstVolumeW(StringBuilder name, uint len);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool FindNextVolumeW(IntPtr h, StringBuilder name, uint len);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FindVolumeClose(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumeInformationW(string root, StringBuilder vn, uint vs, out uint serial, out uint maxc, out uint flags, StringBuilder fsn, uint fsz);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool GetVolumePathNamesForVolumeNameW(string volName, char[] paths, uint len, out uint ret);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandleEx(IntPtr h, int cls, byte[] buf, uint size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);

    static string Paths(string vol) {
        var buf = new char[1024]; uint ret;
        if (!GetVolumePathNamesForVolumeNameW(vol, buf, 1024, out ret) || ret == 0) return "(no path)";
        return string.Join(",", new string(buf, 0, (int)ret).Split(new[] { '\0' }, StringSplitOptions.RemoveEmptyEntries));
    }

    public static string Report() {
        var sb = new StringBuilder();
        var byLow = new Dictionary<uint, List<string>>();
        var name = new StringBuilder(260);
        IntPtr find = FindFirstVolumeW(name, 260);
        if (find == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error(), "FindFirstVolumeW");
        try {
            do {
                string vol = name.ToString();
                string letters = Paths(vol);
                uint s32, mc, fl;
                var vn = new StringBuilder(261); var fs = new StringBuilder(261);
                bool gi = GetVolumeInformationW(vol, vn, 261, out s32, out mc, out fl, fs, 261);
                string fsName = gi ? fs.ToString() : "GetVolumeInformation error " + Marshal.GetLastWin32Error();
                string s64 = "n/a";
                bool hasLow = false; uint low = 0;
                IntPtr h = CreateFileW(vol, 0, 7, IntPtr.Zero, 3, 0x02000000, IntPtr.Zero);
                if (h == new IntPtr(-1)) s64 = "open error " + Marshal.GetLastWin32Error();
                else {
                    try {
                        byte[] buf = new byte[24];
                        if (GetFileInformationByHandleEx(h, 18, buf, 24)) {
                            ulong v = BitConverter.ToUInt64(buf, 0);
                            s64 = "0x" + v.ToString("x16");
                            low = (uint)(v & 0xffffffff); hasLow = true;
                        } else s64 = "FileIdInfo error " + Marshal.GetLastWin32Error();
                    } finally { CloseHandle(h); }
                }
                string low32Match = !hasLow || !gi ? "n/a" : (low == s32 ? "yes" : "NO");
                sb.AppendFormat("{0} [{1}] fs={2} serial32=0x{3:x8} serial64={4} low32==serial32: {5}\n",
                    vol, letters, fsName, gi ? s32 : 0, s64, low32Match);
                if (hasLow) {
                    if (!byLow.ContainsKey(low)) byLow[low] = new List<string>();
                    byLow[low].Add(vol + " " + letters);
                }
            } while (FindNextVolumeW(find, name, 260));
        } finally { FindVolumeClose(find); }
        int collisions = 0;
        foreach (var kv in byLow) {
            if (kv.Value.Count > 1) {
                collisions++;
                sb.AppendFormat("COLLISION low32=0x{0:x8}: {1}\n", kv.Key, string.Join(" | ", kv.Value));
            }
        }
        sb.AppendFormat("volumes with a 64-bit serial: {0}; low-32 collision groups: {1}\n", byLow.Count, collisions);
        return sb.ToString();
    }
}
"@

[PgV2]::Report()
