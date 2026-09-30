# What a Windows binary loads, and whether a PC can be counted on to have it. Dot-sourced by
# windows/scripts/build-core.ps1 (the core and its engines' DLLs) and windows/scripts/pack.ps1 (the
# whole app), so that both hold the release to one rule. Tests:
# windows/scripts/tests/test-dll-imports.ps1.
#
# The rule: every DLL a binary loads when it loads is beside Inkwell.exe (where Windows looks for an
# app's DLLs first) or part of Windows. "Part of Windows" is an API set, or a file of System32 that
# Windows signs as its own (Get-AuthenticodeSignature's IsOSBinary): other software installs into
# System32 too, and a clean PC has none of it (older Visual C++ runtimes such as msvcr120.dll,
# LLVM's OpenMP libomp140.*.dll, C++ AMP's vcamp140.dll). The product name in a file's version
# resource is no test: some of Windows' own name another product (propsys.dll "Windows Search").
# And never one of these, whatever the machine that builds the release has in its System32:
# - vulkan-1.dll, the Vulkan loader, which GPU drivers install;
# - the Visual C++ runtime (vcruntime*, msvcp*, vcomp*, concrt*, mfc*, vccorlib*), which its
#   redistributable installs: the release ships the DLLs of it that the engines' DLLs need beside
#   Inkwell.exe (build-core.ps1 copies them from Visual Studio's redistributable folder), and only
#   that copy counts, built for the app's architecture;
# - ONNX Runtime (onnxruntime*.dll): Windows 11 has an older onnxruntime.dll of its own in System32,
#   and sherpa-onnx must get the one the app ships (ink-engines' src/sherpa.rs also refuses any
#   other version at run time).

Set-StrictMode -Version Latest

# The Visual C++ runtime's DLLs by name (vcruntime140.dll, msvcp140_1.dll, vcomp140.dll, ...; not
# Windows' own msvcp_win.dll or mfcore.dll), and the debug UCRT.
$script:VcRuntimeDll = '^((vcruntime|msvcp|vcomp|concrt|vccorlib|mfcm?)\d[^.]*|ucrtbased)\.dll$'
# The other DLLs that are never part of Windows, whatever a System32 holds.
$script:NotPartOfWindows = '^(vulkan-1|onnxruntime[^.]*)\.dll$'

# Whether $Name (a DLL's file name) is a Visual C++ runtime DLL.
function Test-VcRuntimeDll([string]$Name) {
    return $Name.ToLowerInvariant() -match $script:VcRuntimeDll
}

# The machine a PE file is built for (0x8664 x64, 0xAA64 ARM64): the two bytes after its "PE\0\0"
# signature, which the offset at 0x3C points at. $null for a file that is not a PE file.
function Get-PeMachine([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        if ($stream.Length -lt 0x40 -or $reader.ReadUInt16() -ne 0x5A4D) { return $null }
        $stream.Position = 0x3C
        $pe = $reader.ReadInt32()
        if ($pe -lt 0 -or $pe + 6 -gt $stream.Length) { return $null }
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x4550) { return $null }
        return [int]$reader.ReadUInt16()
    } finally {
        $stream.Dispose()
    }
}

# The Visual C++ runtime DLLs in $Dir and their one file version, as Version (a.b.c.d) and Files
# (lower case, sorted): what a release records it ships. Throws when there is none, or when they
# are not all of one version (DLLs from two redistributables). $VersionOf is for the tests.
function Get-VcRuntimeVersion {
    param(
        [Parameter(Mandatory)] [string]$Dir,
        [scriptblock]$VersionOf = {
            param($path)
            $info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($path)
            '{0}.{1}.{2}.{3}' -f $info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart
        }
    )
    $files = @(Get-ChildItem -LiteralPath $Dir -File | Where-Object { Test-VcRuntimeDll $_.Name } | Sort-Object { $_.Name.ToLowerInvariant() })
    if ($files.Count -eq 0) { throw "no Visual C++ runtime DLL in $Dir" }
    $each = @($files | ForEach-Object { [pscustomobject]@{ Name = $_.Name.ToLowerInvariant(); Version = [string](& $VersionOf $_.FullName) } })
    $versions = @($each | ForEach-Object { $_.Version } | Select-Object -Unique)
    if ($versions.Count -ne 1 -or -not $versions[0]) {
        throw "the Visual C++ runtime DLLs in $Dir are not of one version: $(($each | ForEach-Object { "$($_.Name) $($_.Version)" }) -join ', ')"
    }
    return [pscustomobject]@{ Version = $versions[0]; Files = [string[]]@($each | ForEach-Object { $_.Name }) }
}

# Windows' System32 as a 64-bit program sees it. A 32-bit PowerShell (the first pwsh on some PCs'
# PATH) is shown SysWOW64 there, with the 32-bit DLLs, and reaches the real one as Sysnative.
function Get-System32 {
    if ([Environment]::Is64BitOperatingSystem -and -not [Environment]::Is64BitProcess) {
        return Join-Path $env:SystemRoot 'Sysnative'
    }
    return Join-Path $env:SystemRoot 'System32'
}

# Whether $Name is part of Windows (above). $System32 and $IsOSBinary (whether Windows signs the
# file at a path as its own) are for the tests.
function Test-WindowsDll {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [string]$System32 = (Get-System32),
        [scriptblock]$IsOSBinary = { param($path) (Get-AuthenticodeSignature -LiteralPath $path).IsOSBinary }
    )
    $name = $Name.ToLowerInvariant()
    if ($name -match '^(api|ext)-ms-win-') { return $true }
    if ((Test-VcRuntimeDll $name) -or $name -match $script:NotPartOfWindows) { return $false }
    $path = Join-Path $System32 $name
    return (Test-Path -LiteralPath $path -PathType Leaf) -and [bool](& $IsOSBinary $path)
}

# The DLLs in dumpbin /dependents' output: those loaded when the binary loads (Load), and those
# loaded at the first call into them (Delay), lower case.
function ConvertFrom-DumpbinDependents([string[]]$Lines) {
    $load = [System.Collections.Generic.List[string]]::new()
    $delay = [System.Collections.Generic.List[string]]::new()
    $list = $null
    foreach ($line in $Lines) {
        if ($line -match 'Image has the following dependencies') { $list = $load; continue }
        if ($line -match 'Image has the following delay load dependencies') { $list = $delay; continue }
        if ($line -match '^\s*Summary') { $list = $null; continue }
        if ($null -ne $list -and $line -match '^\s+(\S+\.dll)\s*$') { $list.Add($Matches[1].ToLowerInvariant()) }
    }
    return [pscustomobject]@{
        Load = [string[]]@($load | Select-Object -Unique)
        Delay = [string[]]@($delay | Select-Object -Unique)
    }
}

# The DLLs $Path loads (ConvertFrom-DumpbinDependents), read with Visual Studio's dumpbin.
function Read-DllImports([string]$Dumpbin, [string]$Path) {
    $lines = @(& $Dumpbin /nologo /dependents $Path)
    if ($LASTEXITCODE -ne 0) { throw "dumpbin /dependents failed on $Path ($LASTEXITCODE)" }
    return ConvertFrom-DumpbinDependents $lines
}

# The binaries of $Imports that loading $Roots loads with them: the roots and, transitively, every
# binary of $Imports one of them loads when it loads.
function Get-LoadClosure([hashtable]$Imports, [string[]]$Roots) {
    $seen = [System.Collections.Generic.HashSet[string]]::new()
    $queue = [System.Collections.Generic.Queue[string]]::new()
    foreach ($root in $Roots) { if ($Imports.ContainsKey($root) -and $seen.Add($root)) { $queue.Enqueue($root) } }
    while ($queue.Count -gt 0) {
        foreach ($dll in $Imports[$queue.Dequeue()].Load) {
            if ($Imports.ContainsKey($dll) -and $seen.Add($dll)) { $queue.Enqueue($dll) }
        }
    }
    return [string[]]@($seen)
}

# What a PC may lack, as one line per problem. $Imports maps each binary (its file name, lower
# case) to what it loads (ConvertFrom-DumpbinDependents); $Beside names every file beside
# Inkwell.exe, lower case, and $Machines maps them to the machine each is built for
# (Get-PeMachine); $Machine is the app's. $Roots are the binaries loaded when the app starts
# (Inkwell.exe and the core, which it loads at once); $Core is the core's DLL.
#
# - A DLL loaded when a binary loads must be beside Inkwell.exe or part of Windows
#   (Test-WindowsDll). One exception, the Vulkan loader: a binary the core reaches only through a
#   delay-load of its own may need it. That is the diarizer's Vulkan backend (NeMo-Speech.cpp's
#   ggml-vulkan.dll, behind the delay-loaded nemo_speech_asr_c.dll, which ink-engines' src/nemo.rs
#   checks loads before the first call): on a PC without a Vulkan driver the diarizer does not
#   load, and the rest runs.
# - A DLL the core delay-loads must be beside Inkwell.exe, but for the Vulkan loader (the core then
#   runs llama.cpp on the CPU; build-core.ps1 checks which functions it imports).
# - A Visual C++ runtime DLL, however loaded, must be beside Inkwell.exe and built for the app's
#   machine: never System32's, never another architecture's. Never the debug UCRT (ucrtbased.dll),
#   which Microsoft does not let an app redistribute.
# (Other binaries' delay-loads are not checked: the Windows App SDK delay-loads parts of Windows
# that not every edition has, and handles their absence.)
function Find-ImportProblems {
    param(
        [Parameter(Mandatory)] [hashtable]$Imports,
        [Parameter(Mandatory)] [AllowEmptyCollection()] [string[]]$Beside,
        [Parameter(Mandatory)] [hashtable]$Machines,
        [Parameter(Mandatory)] [int]$Machine,
        [Parameter(Mandatory)] [string[]]$Roots,
        [Parameter(Mandatory)] [string]$Core,
        [scriptblock]$IsWindowsDll = { param($name) Test-WindowsDll -Name $name }
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    # A Visual C++ runtime DLL $binary loads: the problem with it, or nothing.
    $vcRuntime = {
        param($binary, $dll)
        if ($dll -eq 'ucrtbased.dll') {
            "$binary needs ucrtbased.dll, the debug UCRT, which no release may ship"
        } elseif ($Beside -notcontains $dll) {
            "$binary needs $dll, a Visual C++ runtime DLL, which is not beside Inkwell.exe (a PC may not have it; System32's copy on the build machine does not count)"
        } elseif ($Machines[$dll] -ne $Machine) {
            $found = if ($null -eq $Machines[$dll]) { '(unread)' } else { '0x{0:X4}' -f $Machines[$dll] }
            "$binary needs $dll, which is beside Inkwell.exe but built for machine $found, not 0x{0:X4}" -f $Machine
        }
    }
    $atStart = Get-LoadClosure $Imports $Roots
    $delayed = if ($Imports.ContainsKey($Core)) { $Imports[$Core].Delay } else { @() }
    $deferred = @(Get-LoadClosure $Imports $delayed | Where-Object { $atStart -notcontains $_ })
    foreach ($binary in @($Imports.Keys | Sort-Object)) {
        foreach ($dll in $Imports[$binary].Load) {
            if (Test-VcRuntimeDll $dll) {
                $problem = & $vcRuntime $binary $dll
                if ($problem) { $problems.Add($problem) }
            } elseif ($Beside -contains $dll -or (& $IsWindowsDll $dll)) {
                continue
            } elseif ($dll -eq 'vulkan-1.dll' -and $deferred -contains $binary) {
                continue
            } elseif ($dll -eq 'vulkan-1.dll') {
                $problems.Add("$binary needs vulkan-1.dll when it loads: without a Vulkan driver the app would not start")
            } else {
                $problems.Add("$binary needs $dll when it loads, and it is neither beside Inkwell.exe nor part of Windows")
            }
        }
        foreach ($dll in $Imports[$binary].Delay) {
            if (Test-VcRuntimeDll $dll) {
                $problem = & $vcRuntime $binary $dll
                if ($problem) { $problems.Add($problem) }
            } elseif ($binary -eq $Core -and $dll -ne 'vulkan-1.dll' -and $Beside -notcontains $dll) {
                $problems.Add("$binary delay-loads $dll, which is not beside Inkwell.exe")
            }
        }
    }
    return [string[]]@($problems)
}
