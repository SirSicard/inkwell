# What a Windows binary loads, and whether a PC can be counted on to have it. Dot-sourced by
# windows/scripts/build-core.ps1 (the core and its engines' DLLs) and windows/scripts/pack.ps1 (the
# whole app), so that both hold the release to one rule. Tests:
# windows/scripts/tests/test-dll-imports.ps1.
#
# The rule: every DLL a binary loads when it loads is beside Inkwell.exe (where Windows looks for an
# app's DLLs first) or part of Windows. "Part of Windows" is an API set, or a file of System32 that
# is not one of these, whatever the machine that builds the release has in its System32:
# - vulkan-1.dll, the Vulkan loader, which GPU drivers install;
# - the Visual C++ runtime (vcruntime*, msvcp*, vcomp*, concrt*, mfc*, vccorlib*), which its
#   redistributable installs, and which the release neither needs nor ships;
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

# Windows' System32 as a 64-bit program sees it. A 32-bit PowerShell (the first pwsh on some PCs'
# PATH) is shown SysWOW64 there, with the 32-bit DLLs, and reaches the real one as Sysnative.
function Get-System32 {
    if ([Environment]::Is64BitOperatingSystem -and -not [Environment]::Is64BitProcess) {
        return Join-Path $env:SystemRoot 'Sysnative'
    }
    return Join-Path $env:SystemRoot 'System32'
}

# Whether $Name is part of Windows (above). $System32 is for the tests.
function Test-WindowsDll {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [string]$System32 = (Get-System32)
    )
    $name = $Name.ToLowerInvariant()
    if ($name -match '^(api|ext)-ms-win-') { return $true }
    if ((Test-VcRuntimeDll $name) -or $name -match $script:NotPartOfWindows) { return $false }
    return Test-Path -LiteralPath (Join-Path $System32 $name) -PathType Leaf
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
# Inkwell.exe, lower case. $Roots are the binaries loaded when the app starts (Inkwell.exe and the
# core, which it loads at once); $Core is the core's DLL.
#
# - A DLL loaded when a binary loads must be beside Inkwell.exe or part of Windows
#   (Test-WindowsDll). One exception, the Vulkan loader: a binary the core reaches only through a
#   delay-load of its own may need it. That is the diarizer's Vulkan backend (NeMo-Speech.cpp's
#   ggml-vulkan.dll, behind the delay-loaded nemo_speech_asr_c.dll, which ink-engines' src/nemo.rs
#   checks loads before the first call): on a PC without a Vulkan driver the diarizer does not
#   load, and the rest runs.
# - A DLL the core delay-loads must be beside Inkwell.exe, but for the Vulkan loader (the core then
#   runs llama.cpp on the CPU; build-core.ps1 checks which functions it imports).
# - A Visual C++ runtime DLL is never allowed, however loaded.
# (Other binaries' delay-loads are not checked: the Windows App SDK delay-loads parts of Windows
# that not every edition has, and handles their absence.)
function Find-ImportProblems {
    param(
        [Parameter(Mandatory)] [hashtable]$Imports,
        [Parameter(Mandatory)] [AllowEmptyCollection()] [string[]]$Beside,
        [Parameter(Mandatory)] [string[]]$Roots,
        [Parameter(Mandatory)] [string]$Core,
        [scriptblock]$IsWindowsDll = { param($name) Test-WindowsDll -Name $name }
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    $atStart = Get-LoadClosure $Imports $Roots
    $delayed = if ($Imports.ContainsKey($Core)) { $Imports[$Core].Delay } else { @() }
    $deferred = @(Get-LoadClosure $Imports $delayed | Where-Object { $atStart -notcontains $_ })
    foreach ($binary in @($Imports.Keys | Sort-Object)) {
        foreach ($dll in $Imports[$binary].Load) {
            if (Test-VcRuntimeDll $dll) {
                $problems.Add("$binary needs $dll, a Visual C++ runtime DLL: a PC may not have it, and the release ships none")
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
                $problems.Add("$binary needs $dll, a Visual C++ runtime DLL: a PC may not have it, and the release ships none")
            } elseif ($binary -eq $Core -and $dll -ne 'vulkan-1.dll' -and $Beside -notcontains $dll) {
                $problems.Add("$binary delay-loads $dll, which is not beside Inkwell.exe")
            }
        }
    }
    return [string[]]@($problems)
}
