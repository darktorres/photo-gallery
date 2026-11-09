# Builds FFmpeg 9.0 as static libraries with MSVC, for pg-video and Qt Multimedia's FFmpeg backend.
#
# Reproducible: the inputs are pinned below (the FFmpeg commit, the MSVC toolset and Windows SDK, the MSYS2 packages),
# and each one is checked before it is used. The source is fetched by commit hash. Every step's log and the outputs'
# hashes go to <Work>\manifest.json. The reasons for each step are in docs/STATIC_BUILD.md.
#
#   pwsh -File tools/build-static-ffmpeg.ps1                       # into D:\PhotoGallery\static\ffmpeg
#   pwsh -File tools/build-static-ffmpeg.ps1 -Work D:\scratch\run2  # a clean run, into another folder
#
# Not a Windows-only shortcut: the build needs MSVC and MSYS2 (for FFmpeg's configure and make), both on this machine.

param(
    [string]$Work = 'D:\PhotoGallery\static\ffmpeg',
    [string]$BuildTools = 'C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools',
    [string]$Msys = 'C:\tools\msys64',
    [int]$Jobs = [Environment]::ProcessorCount
)

$ErrorActionPreference = 'Stop'

# ---- Pins. Change them here, and record the reason in docs/STATIC_BUILD.md. --------------------------------------
$FFmpegCommit  = '7adfbe40dcde598f93a6963d2f7b22cd75202ccc'   # n9.0.2-39-g7adfbe40dc, the commit the pinned build came from
$FFmpegRelease = '9.0.2'                                      # the RELEASE file at that commit
$Toolset       = '14.44.35207'                                # MSVC toolset (vcvars64 -vcvars_ver)
$WindowsSdk    = '10.0.26100.0'
$MsysPackages  = @{ make = '4.4.1-3'; diffutils = '3.12-1' } # installed if missing; a different version is recorded
# The Windows libraries the static link needs. FFmpeg's .pc files don't list them (Libs.private is empty).
$SystemLibs    = @('user32.lib', 'bcrypt.lib')

# The configure line. --prefix is added below. Each flag is the pilot's, see docs/STATIC_BUILD.md.
$ConfigureFlags = @(
    '--toolchain=msvc', '--enable-static', '--disable-shared',
    "--extra-cflags='-MD -Brepro'",                          # the app's dynamic CRT (MSVCRT); -Brepro: same bytes per build
    '--disable-debug',                                       # no /Z7: its debug data embeds the build folder's source paths
    '--disable-programs', '--disable-doc', '--disable-autodetect', '--disable-network',
    '--disable-avdevice', '--disable-avfilter', '--disable-everything',
    '--enable-protocol=file',
    '--enable-demuxer=mov,matroska,avi,mpegts,image2',
    '--enable-decoder=h264,hevc,vp9,mpeg4,mjpeg',
    '--enable-parser=h264,hevc,vp9,mpeg4video,mjpeg',
    '--enable-swscale', '--enable-swresample',
    '--enable-d3d11va', '--enable-dxva2'
)

# ---- Helpers ----------------------------------------------------------------------------------------------------
function Invoke-Checked([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

# D:\a\b -> /d/a/b, for MSYS2's bash.
function ConvertTo-Posix([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    '/' + $full.Substring(0, 1).ToLower() + $full.Substring(2).Replace('\', '/')
}

function Import-MsvcEnvironment {
    $vcvars = Join-Path $BuildTools 'VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path $vcvars)) { throw "vcvars64.bat not found: $vcvars (Visual Studio Build Tools, see the pins)" }
    $dump = & cmd.exe /d /c "`"$vcvars`" -vcvars_ver=$Toolset >nul 2>nul && set"
    if ($LASTEXITCODE -ne 0) { throw "vcvars64.bat failed (exit $LASTEXITCODE)" }
    foreach ($line in $dump) {
        $i = $line.IndexOf('=')
        if ($i -gt 0) { Set-Item -Path "env:$($line.Substring(0, $i))" -Value $line.Substring($i + 1) }
    }
    # MSYS2's usr\bin after the MSVC tools: MSYS2's link (coreutils) would otherwise shadow MSVC's link.exe.
    $cl = (Get-Command cl.exe -ErrorAction Stop).Source
    if ($cl -notlike "*\$($Toolset)\*") { throw "cl.exe is not toolset ${Toolset}: $cl" }
    $env:PATH = "$(Split-Path $cl);$Msys\usr\bin;$env:PATH"
    $env:MSYS2_PATH_TYPE = 'inherit'
    if (-not (Test-Path "${env:ProgramFiles(x86)}\Windows Kits\10\Include\$WindowsSdk\um\dxva.h")) {
        throw "Windows SDK $WindowsSdk not installed (dxva.h missing)"
    }
    return $cl
}

function Assert-MsysPackages {
    $pacman = Join-Path $Msys 'usr\bin\pacman.exe'
    if (-not (Test-Path $pacman)) { throw "MSYS2 not found at $Msys" }
    $installed = @{}
    foreach ($name in $MsysPackages.Keys) {
        $line = & $pacman -Q $name 2>$null
        if ($LASTEXITCODE -eq 0 -and $line) { $installed[$name] = ($line -split '\s+')[1] }
    }
    $missing = @($MsysPackages.Keys | Where-Object { -not $installed.ContainsKey($_) })
    if ($missing) {
        Invoke-Checked 'pacman -Sy' { & $pacman -Sy --noconfirm *> (Join-Path $Work 'logs\pacman-sync.log') }
        Invoke-Checked 'pacman -S' { & $pacman -S --needed --noconfirm @missing *> (Join-Path $Work 'logs\pacman-install.log') }
        foreach ($name in $missing) { $installed[$name] = (& $pacman -Q $name) -split '\s+' | Select-Object -Index 1 }
    }
    $different = @($MsysPackages.Keys | Where-Object { $installed[$_] -ne $MsysPackages[$_] })
    if ($different) {
        Write-Warning ("MSYS2 packages differ from the pins: " + (($different | ForEach-Object { "$_ $($installed[$_]) (pinned $($MsysPackages[$_]))" }) -join ', '))
    }
    return $installed
}

# ---- Build ------------------------------------------------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $Work, "$Work\logs", "$Work\test" | Out-Null
$Work = (Resolve-Path $Work).Path
$Src = Join-Path $Work 'src'
$Prefix = Join-Path $Work 'prefix'
$started = Get-Date

$cl = Import-MsvcEnvironment
$msysVersions = Assert-MsysPackages
"cl: $cl"

# 1. Source, by commit.
if (-not (Test-Path "$Src\.git")) { Invoke-Checked 'git init' { git init -q $Src } }
Push-Location $Src
try {
    if (-not (git remote 2>$null | Select-String -Quiet '^origin$')) { git remote add origin https://github.com/FFmpeg/FFmpeg.git }
    # The network drops connections now and then: retry the fetch a few times before failing.
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        git fetch -q --depth 1 origin $FFmpegCommit *> "$Work\logs\git-fetch.log"
        if ($LASTEXITCODE -eq 0) { break }
        if ($attempt -eq 3) { throw "git fetch failed after 3 attempts (exit $LASTEXITCODE): see $Work\logs\git-fetch.log" }
        Start-Sleep -Seconds (10 * $attempt)
    }
    Invoke-Checked 'git checkout' { git checkout -q FETCH_HEAD }
    $head = (git rev-parse HEAD).Trim()
    if ($head -ne $FFmpegCommit) { throw "source HEAD is $head, not the pin $FFmpegCommit" }
    # The build is in-tree: start from the pinned commit with nothing left from an earlier build.
    git clean -fdxq
    git reset -q --hard $FFmpegCommit
    $release = (Get-Content RELEASE).Trim()
    if ($release -ne $FFmpegRelease) { throw "RELEASE is $release, not $FFmpegRelease" }
} finally { Pop-Location }

# 2. Configure, in the source folder (an in-tree build). Two reasons, both from the reproducibility runs:
#    - __FILE__ (assertions, log messages) gets the source path as passed to the compiler: relative in-tree, absolute
#      out of tree, and the absolute one would put the work folder into every library.
#    - FFMPEG_CONFIGURATION, compiled into avutil, holds --prefix. The prefix is fixed (/ffbuild/prefix, as the pinned
#      build has it) and install goes to a staging folder with DESTDIR, which doesn't appear in the libraries.
$flags = $ConfigureFlags -join ' '
$staged = Join-Path $Work 'staged'
$configureSh = Join-Path $Work 'configure.sh'
$configureText = @"
#!/bin/sh
set -eo pipefail
cd '$(ConvertTo-Posix $Src)'
./configure $flags --prefix=/ffbuild/prefix 2>&1 | tee '$(ConvertTo-Posix $Work)/logs/configure.log'
"@
[IO.File]::WriteAllText($configureSh, $configureText.Replace("`r`n", "`n"))
$bash = Join-Path $Msys 'usr\bin\bash.exe'
Invoke-Checked 'configure' { & $bash --noprofile --norc (ConvertTo-Posix $configureSh) }

$configureLog = Get-Content "$Work\logs\configure.log" -Raw
if ($configureLog -match 'cmp: command not found') { throw 'configure ran without cmp (diffutils missing): the result cannot be trusted' }
$disabled = @([regex]::Matches($configureLog, 'WARNING: Disabled [^\r\n]+') | ForEach-Object Value)
$license = [regex]::Match($configureLog, 'License: [^\r\n]+').Value

# 3. Build and install.
$buildSh = Join-Path $Work 'build.sh'
$buildText = @"
#!/bin/sh
set -eo pipefail
cd '$(ConvertTo-Posix $Src)'
make -j$Jobs ARFLAGS='-nologo -Brepro' 2>&1 | tee '$(ConvertTo-Posix $Work)/logs/build.log'
make install DESTDIR='$(ConvertTo-Posix $staged)' 2>&1 | tee '$(ConvertTo-Posix $Work)/logs/install.log'
"@
[IO.File]::WriteAllText($buildSh, $buildText.Replace("`r`n", "`n"))
Invoke-Checked 'make' { & $bash --noprofile --norc (ConvertTo-Posix $buildSh) }

# The install went to <staged>\ffbuild\prefix (the fixed prefix under DESTDIR). Move it to the work folder's prefix.
$stagedPrefix = Join-Path $staged 'ffbuild\prefix'
if (-not (Test-Path (Join-Path $stagedPrefix 'lib'))) { throw "install produced nothing under $stagedPrefix" }
if (Test-Path $Prefix) { & cmd.exe /d /c "rmdir /s /q `"$Prefix`"" }
Move-Item -LiteralPath $stagedPrefix -Destination $Prefix
& cmd.exe /d /c "rmdir /s /q `"$staged`""

# 4. The CRT: the libraries must use the dynamic runtime (MSVCRT), never the static one (LIBCMT).
$libs = Get-ChildItem "$Prefix\lib\*.lib" | Sort-Object Name
$crt = foreach ($lib in $libs) {
    $bytes = [IO.File]::ReadAllBytes($lib.FullName)
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    [pscustomobject]@{ Name = $lib.Name; MSVCRT = ([regex]::Matches($text, 'MSVCRT')).Count; LIBCMT = ([regex]::Matches($text, 'LIBCMT')).Count }
}
if (@($crt | Where-Object LIBCMT -gt 0).Count -gt 0) { throw 'a library references LIBCMT (static CRT): see the manifest' }

# 5. Link check: a program that calls into each library, linked with the system libraries, then run.
Copy-Item (Join-Path $PSScriptRoot 'static-ffmpeg\static-check.c') "$Work\test\static-check.c" -Force
$libArgs = @('avformat.lib', 'avcodec.lib', 'avutil.lib', 'swresample.lib', 'swscale.lib') + $SystemLibs
Push-Location "$Work\test"
try {
    Invoke-Checked 'cl (link check)' { & cl.exe /nologo /MD /W3 "/I$Prefix\include" static-check.c /Fe:static-check.exe /link "/LIBPATH:$Prefix\lib" @libArgs *> "$Work\logs\link-check.log" }
    $output = & .\static-check.exe
    if ($LASTEXITCODE -ne 0) { throw "static-check.exe exited $LASTEXITCODE" }
    $expected = @('h264 decoder: yes', 'd3d11va hwdevice: yes', 'dxva2 hwdevice: yes', 'open missing file: -2')
    foreach ($line in $expected) { if (-not ($output | Select-String -SimpleMatch $line)) { throw "static-check output lacks: $line" } }
    $dependents = & dumpbin.exe /nologo /dependents static-check.exe | Where-Object { $_ -match '^\s+\S+\.dll$' } | ForEach-Object { $_.Trim() }
} finally { Pop-Location }
$nonSystem = @($dependents | Where-Object { $_ -notmatch '^(api-ms-win-|KERNEL32|USER32|bcrypt|VCRUNTIME140|MSVCP140)' })

# 6. Manifest: what went in and what came out, with hashes.
$manifest = [ordered]@{
    built           = $started.ToUniversalTime().ToString('o')
    seconds         = [int]((Get-Date) - $started).TotalSeconds
    ffmpeg          = [ordered]@{ commit = $head; release = $release; configure = "./configure $flags --prefix=/ffbuild/prefix" }
    tools           = [ordered]@{
        cl            = $cl
        msvc_toolset  = $Toolset
        windows_sdk   = $WindowsSdk
        msys2_pacman  = ($msysVersions.GetEnumerator() | ForEach-Object { "$($_.Key) $($_.Value)" })
        msys2_root    = $Msys
    }
    configure       = [ordered]@{ license = $license; disabled_with_warning = $disabled }
    libraries       = @($libs | ForEach-Object {
        [ordered]@{ name = $_.Name; bytes = $_.Length; sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash }
    })
    crt             = $crt
    link_check      = [ordered]@{ output = @($output); dependents = $dependents; non_system = $nonSystem }
}
$manifest | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 "$Work\manifest.json"
"done: $Work\manifest.json"
if ($nonSystem.Count -gt 0) { Write-Warning "the link check imports non-system DLLs: $($nonSystem -join ', ')" }
