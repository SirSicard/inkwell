#!/usr/bin/env pwsh
# windows/scripts/lib/dll-imports.ps1: which DLLs count as part of Windows, how dumpbin's output is
# read, and what a package may not need. No dumpbin and no Windows needed (a System32 is faked):
#
#   pwsh windows/scripts/tests/test-dll-imports.ps1
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../lib/dll-imports.ps1')

$script:failures = 0
function Check([string]$Label, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) { Write-Output "  ok    $Label" } else { Write-Output "  FAIL  $Label $Detail"; $script:failures++ }
}

# A System32 holding a few of Windows' own DLLs, and what a build machine's also holds: Windows'
# older ONNX Runtime, a GPU driver's Vulkan loader, the Visual C++ runtime, and other
# redistributables (an older Visual C++ runtime, LLVM's OpenMP, C++ AMP). Which files Windows signs
# as its own is faked too: the real check reads each file's signature.
$system32 = Join-Path ([System.IO.Path]::GetTempPath()) "ink-dll-imports-$PID"
New-Item -ItemType Directory -Force $system32 | Out-Null
try {
    $windowsOwn = @('kernel32.dll', 'dxgi.dll', 'dbghelp.dll', 'msvcp_win.dll', 'mfcore.dll', 'ucrtbase.dll', 'onnxruntime.dll')
    foreach ($name in @($windowsOwn) + @('vulkan-1.dll', 'vcruntime140.dll', 'msvcp140.dll', 'vcomp140.dll',
            'msvcr120.dll', 'libomp140.aarch64.dll', 'vcamp140.dll')) {
        New-Item -ItemType File (Join-Path $system32 $name) | Out-Null
    }
    $signedAsWindows = { param($path) $windowsOwn -contains (Split-Path -Leaf $path) }
    $partOfWindows = { param($name) Test-WindowsDll -Name $name -System32 $system32 -IsOSBinary $signedAsWindows }

    Write-Output 'part of Windows'
    foreach ($name in 'kernel32.dll', 'KERNEL32.dll', 'dxgi.dll', 'msvcp_win.dll', 'mfcore.dll', 'ucrtbase.dll',
        'api-ms-win-crt-heap-l1-1-0.dll', 'ext-ms-win-ntuser-window-l1-1-0.dll') {
        Check "$name is" (& $partOfWindows $name)
    }
    foreach ($name in 'onnxruntime.dll', 'ONNXRUNTIME.DLL', 'onnxruntime_providers_shared.dll', 'vulkan-1.dll',
        'vcruntime140.dll', 'msvcp140.dll', 'vcomp140.dll', 'sherpa-onnx-c-api.dll', 'ggml.dll') {
        Check "$name is not, whatever System32 holds" (-not (& $partOfWindows $name))
    }
    foreach ($name in 'msvcr120.dll', 'libomp140.aarch64.dll', 'vcamp140.dll') {
        Check "$name is not: in System32, but not Windows' own" (-not (& $partOfWindows $name))
    }

    Write-Output 'the Visual C++ runtime'
    foreach ($name in 'vcruntime140.dll', 'VCRUNTIME140_1.dll', 'msvcp140.dll', 'MSVCP140_1.dll', 'VCOMP140.DLL',
        'concrt140.dll', 'vccorlib140.dll', 'mfc140u.dll', 'mfcm140.dll', 'ucrtbased.dll') {
        Check "$name is" (Test-VcRuntimeDll $name)
    }
    foreach ($name in 'msvcp_win.dll', 'mfcore.dll', 'mfplat.dll', 'ucrtbase.dll', 'vulkan-1.dll', 'kernel32.dll') {
        Check "$name is not" (-not (Test-VcRuntimeDll $name))
    }

    Write-Output "dumpbin's output"
    $read = ConvertFrom-DumpbinDependents @(
        'Dump of file C:\app\ink_ffi.dll', '', 'File Type: DLL', '',
        '  Image has the following dependencies:', '',
        '    sherpa-onnx-c-api.dll', '    onnxruntime.dll', '    KERNEL32.dll', '    api-ms-win-core-synch-l1-2-0.dll', '',
        '  Image has the following delay load dependencies:', '',
        '    vulkan-1.dll', '    nemo_speech_asr_c.dll', '',
        '  Summary', '', '       1000 .data', '     A5000 .text')
    Check 'load-time imports, lower case' (($read.Load -join ',') -eq 'sherpa-onnx-c-api.dll,onnxruntime.dll,kernel32.dll,api-ms-win-core-synch-l1-2-0.dll') ($read.Load -join ',')
    Check 'delay-loads' (($read.Delay -join ',') -eq 'vulkan-1.dll,nemo_speech_asr_c.dll') ($read.Delay -join ',')
    $none = ConvertFrom-DumpbinDependents @('Dump of file C:\app\resources.dll', '', 'File Type: DLL', '', '  Summary', '', '       1000 .rsrc')
    Check 'a DLL that imports nothing' ($none.Load.Count -eq 0 -and $none.Delay.Count -eq 0)

    Write-Output 'what a package may need'
    function Deps([string[]]$Load, [string[]]$Delay = @()) { [pscustomobject]@{ Load = $Load; Delay = $Delay } }
    # The x64 release: sherpa-onnx at load time, NeMo delay-loaded, its Vulkan backend behind it,
    # and the Visual C++ runtime the engines' DLLs need, beside the app.
    function Package {
        @{
            'inkwell.exe' = Deps @('kernel32.dll', 'api-ms-win-crt-heap-l1-1-0.dll')
            'ink_ffi.dll' = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll') @('vulkan-1.dll', 'nemo_speech_asr_c.dll')
            'sherpa-onnx-c-api.dll' = Deps @('onnxruntime.dll', 'kernel32.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
            'onnxruntime.dll' = Deps @('kernel32.dll', 'dxgi.dll', 'dbghelp.dll', 'msvcp140.dll', 'vcruntime140.dll')
            'onnxruntime_providers_shared.dll' = Deps @('kernel32.dll', 'vcruntime140.dll')
            'nemo_speech_asr_c.dll' = Deps @('nemo_speech_asr.dll', 'kernel32.dll', 'vcruntime140.dll')
            'nemo_speech_asr.dll' = Deps @('ggml.dll', 'ggml-base.dll', 'kernel32.dll', 'msvcp140.dll', 'msvcp140_1.dll')
            'ggml.dll' = Deps @('ggml-cpu.dll', 'ggml-vulkan.dll', 'ggml-base.dll')
            'ggml-base.dll' = Deps @('kernel32.dll')
            'ggml-cpu.dll' = Deps @('ggml-base.dll', 'kernel32.dll', 'vcomp140.dll')
            'ggml-vulkan.dll' = Deps @('ggml-base.dll', 'vulkan-1.dll', 'kernel32.dll')
            # The runtime's own DLLs: the UCRT (part of Windows) and each other.
            'vcruntime140.dll' = Deps @('kernel32.dll', 'api-ms-win-crt-runtime-l1-1-0.dll')
            'vcruntime140_1.dll' = Deps @('vcruntime140.dll', 'kernel32.dll')
            'msvcp140.dll' = Deps @('vcruntime140.dll', 'kernel32.dll', 'api-ms-win-crt-heap-l1-1-0.dll')
            'msvcp140_1.dll' = Deps @('msvcp140.dll', 'vcruntime140.dll')
            'vcomp140.dll' = Deps @('kernel32.dll', 'api-ms-win-crt-heap-l1-1-0.dll')
            # The Windows App SDK delay-loads parts of Windows an edition may lack.
            'microsoft.ui.xaml.dll' = Deps @('kernel32.dll') @('ext-ms-win-something-l1-1-0.dll', 'not-on-this-edition.dll')
        }
    }
    # Every file beside the app built for x64 (0x8664), but those $Other names, built for ARM64.
    function Machines([string[]]$Beside, [string[]]$Other = @()) {
        $map = @{}
        foreach ($name in $Beside) { $map[$name] = if ($Other -contains $name) { 0xAA64 } else { 0x8664 } }
        return $map
    }
    function Problems([hashtable]$Imports, [string[]]$Beside = @($Imports.Keys), [hashtable]$Machines = (Machines $Beside)) {
        Find-ImportProblems -Imports $Imports -Beside $Beside -Machines $Machines -Machine 0x8664 -Roots 'inkwell.exe', 'ink_ffi.dll' -Core 'ink_ffi.dll' -IsWindowsDll $partOfWindows
    }
    $ok = @(Problems (Package))
    Check 'the release as built: nothing missing' ($ok.Count -eq 0) ($ok -join '; ')

    $p = Package
    $beside = @($p.Keys | Where-Object { $_ -ne 'onnxruntime.dll' })
    $found = @(Problems $p $beside)
    Check "ONNX Runtime not beside the app: System32's does not count" ($found.Count -eq 2 -and ($found -join ';') -match 'ink_ffi.dll needs onnxruntime.dll' -and ($found -join ';') -match 'sherpa-onnx-c-api.dll needs onnxruntime.dll') ($found -join '; ')

    $p = Package
    $p['ink_ffi.dll'] = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll', 'nemo_speech_asr_c.dll') @('vulkan-1.dll')
    $found = @(Problems $p)
    Check 'NeMo loaded with the core: its Vulkan backend would stop the app without a driver' ($found.Count -eq 1 -and $found[0] -match '^ggml-vulkan.dll needs vulkan-1.dll when it loads') ($found -join '; ')

    $core = @{
        'inkwell.exe' = Deps @('kernel32.dll')
        'ink_ffi.dll' = Deps @('kernel32.dll', 'vulkan-1.dll')
    }
    $found = @(Problems $core)
    Check 'the core itself loading Vulkan at load time' ($found.Count -eq 1 -and $found[0] -match '^ink_ffi.dll needs vulkan-1.dll when it loads') ($found -join '; ')

    # The diarizer's DLLs left out of the package (what Inkwell.csproj once did).
    $p = Package
    foreach ($name in 'nemo_speech_asr_c.dll', 'nemo_speech_asr.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll', 'ggml-vulkan.dll') { $p.Remove($name) }
    $found = @(Problems $p)
    Check 'a delay-loaded engine not beside the app' ($found.Count -eq 1 -and $found[0] -match '^ink_ffi.dll delay-loads nemo_speech_asr_c.dll') ($found -join '; ')

    $p = Package
    $found = @(Problems $p @($p.Keys | Where-Object { $_ -ne 'ggml-cpu.dll' }))
    Check 'a DLL an engine needs, missing' ($found.Count -eq 1 -and $found[0] -match '^ggml.dll needs ggml-cpu.dll when it loads, and it is neither beside') ($found -join '; ')

    # ggml built with LLVM's OpenMP, whose runtime the build machine's System32 happens to hold.
    $p = Package
    $p['ggml-cpu.dll'] = Deps @('ggml-base.dll', 'kernel32.dll', 'libomp140.aarch64.dll') @()
    $found = @(Problems $p)
    Check "a redistributable in the build machine's System32" ($found.Count -eq 1 -and $found[0] -match '^ggml-cpu.dll needs libomp140.aarch64.dll when it loads, and it is neither beside') ($found -join '; ')

    # The Visual C++ runtime: only the copy beside the app counts, built for the app's architecture.
    # The build machine's System32 has one (its redistributable), which a PC may lack.
    $p = Package
    $found = @(Problems $p @($p.Keys | Where-Object { $_ -notin 'vcomp140.dll', 'msvcp140_1.dll' }))
    Check "the Visual C++ runtime not beside the app: System32's does not count" ($found.Count -eq 2 -and ($found -join ';') -match 'ggml-cpu.dll needs vcomp140.dll, a Visual C\+\+ runtime DLL, which is not beside Inkwell.exe' -and ($found -join ';') -match 'nemo_speech_asr.dll needs msvcp140_1.dll, a Visual C\+\+ runtime DLL, which is not beside') ($found -join '; ')

    $p = Package
    $found = @(Problems $p @($p.Keys) (Machines @($p.Keys) 'msvcp140.dll'))
    Check "the Visual C++ runtime beside the app, but another architecture's" ($found.Count -eq 4 -and ($found -join ';') -match 'onnxruntime.dll needs msvcp140.dll, which is beside Inkwell.exe but built for machine 0xAA64, not 0x8664' -and ($found -join ';') -match 'msvcp140_1.dll needs msvcp140.dll, which is beside') ($found -join '; ')

    $p = Package
    $machines = Machines @($p.Keys)
    $machines.Remove('vcruntime140_1.dll')
    $found = @(Problems $p @($p.Keys) $machines)
    Check "a Visual C++ runtime DLL beside the app whose architecture was not read" ($found.Count -eq 1 -and $found[0] -match 'sherpa-onnx-c-api.dll needs vcruntime140_1.dll, which is beside Inkwell.exe but built for machine \(unread\), not 0x8664') ($found -join '; ')

    $p = Package
    $p['sherpa-onnx-c-api.dll'] = Deps @('onnxruntime.dll', 'vcruntime140.dll') @('msvcp140.dll', 'concrt140.dll')
    $found = @(Problems $p @($p.Keys))
    Check 'a delay-loaded Visual C++ runtime DLL: the same rule' ($found.Count -eq 1 -and $found[0] -match 'sherpa-onnx-c-api.dll needs concrt140.dll, a Visual C\+\+ runtime DLL, which is not beside') ($found -join '; ')

    $p = Package
    $p['ggml-cpu.dll'] = Deps @('ggml-base.dll', 'ucrtbased.dll') @()
    $found = @(Problems $p @(@($p.Keys) + 'ucrtbased.dll'))
    Check 'the debug UCRT, even beside the app' ($found.Count -eq 1 -and $found[0] -match 'ggml-cpu.dll needs ucrtbased.dll, the debug UCRT, which no release may ship') ($found -join '; ')

    # A core without the diarizer: no delay-load at all.
    $arm = @{
        'inkwell.exe' = Deps @('kernel32.dll')
        'ink_ffi.dll' = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll')
        'sherpa-onnx-c-api.dll' = Deps @('onnxruntime.dll')
        'onnxruntime.dll' = Deps @('kernel32.dll')
    }
    $found = @(Problems $arm)
    Check 'a core without the diarizer' ($found.Count -eq 0) ($found -join '; ')
    $arm['ink_ffi.dll'] = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll') @('vulkan-1.dll')
    Check "a delay-loaded Vulkan loader may be missing" (@(Problems $arm).Count -eq 0)

    Write-Output 'PE machines'
    # The two bytes after "PE\0\0", found through the offset at 0x3C.
    function PeFile([string]$Name, [int]$Machine) {
        $bytes = [byte[]]::new(0x90)
        $bytes[0] = 0x4D; $bytes[1] = 0x5A
        [BitConverter]::GetBytes([int32]0x80).CopyTo($bytes, 0x3C)
        $bytes[0x80] = 0x50; $bytes[0x81] = 0x45
        [BitConverter]::GetBytes([uint16]$Machine).CopyTo($bytes, 0x84)
        $path = Join-Path $system32 $Name
        [System.IO.File]::WriteAllBytes($path, $bytes)
        return $path
    }
    Check 'x64' ((Get-PeMachine (PeFile 'x64.dll' 0x8664)) -eq 0x8664)
    Check 'ARM64' ((Get-PeMachine (PeFile 'arm64.dll' 0xAA64)) -eq 0xAA64)
    $text = Join-Path $system32 'not-pe.dll'
    Set-Content -LiteralPath $text -Value 'not a PE file'
    Check 'not a PE file: none' ($null -eq (Get-PeMachine $text))

    Write-Output "the Visual C++ runtime's version"
    $dir = Join-Path $system32 'app'
    New-Item -ItemType Directory $dir | Out-Null
    foreach ($name in 'Inkwell.exe', 'ink_ffi.dll', 'vcruntime140.dll', 'msvcp140.dll', 'VCOMP140.dll') { New-Item -ItemType File (Join-Path $dir $name) | Out-Null }
    $versions = @{ 'vcruntime140.dll' = '14.50.35719.0'; 'msvcp140.dll' = '14.50.35719.0'; 'VCOMP140.dll' = '14.50.35719.0'; 'ink_ffi.dll' = '1.0.0.0' }
    $versionOf = { param($path) $versions[(Split-Path -Leaf $path)] }
    $runtime = Get-VcRuntimeVersion -Dir $dir -VersionOf $versionOf
    Check 'one version, over the runtime DLLs alone' ($runtime.Version -eq '14.50.35719.0') $runtime.Version
    Check '... and their names, lower case and sorted' (($runtime.Files -join ',') -eq 'msvcp140.dll,vcomp140.dll,vcruntime140.dll') ($runtime.Files -join ',')
    $versions['msvcp140.dll'] = '14.44.35211.0'
    $threw = $null
    try { Get-VcRuntimeVersion -Dir $dir -VersionOf $versionOf | Out-Null } catch { $threw = "$_" }
    Check 'two versions mixed: refused' ($threw -match 'not of one version: msvcp140.dll 14.44.35211.0, vcomp140.dll 14.50.35719.0, vcruntime140.dll 14.50.35719.0') $threw
    Remove-Item (Join-Path $dir 'vcruntime140.dll'), (Join-Path $dir 'msvcp140.dll'), (Join-Path $dir 'VCOMP140.dll')
    $threw = $null
    try { Get-VcRuntimeVersion -Dir $dir -VersionOf $versionOf | Out-Null } catch { $threw = "$_" }
    Check 'none at all: refused' ($threw -match 'no Visual C\+\+ runtime DLL') $threw

    Write-Output 'load closures'
    function Same([string[]]$A, [string[]]$B) { (($A | Sort-Object) -join ',') -eq (($B | Sort-Object) -join ',') }
    $closure = @(Get-LoadClosure (Package) @('ink_ffi.dll'))
    Check "the core's load-time closure stops at its delay-loads" (Same $closure 'ink_ffi.dll', 'onnxruntime.dll', 'sherpa-onnx-c-api.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') ($closure -join ',')
    $closure = @(Get-LoadClosure (Package) @('nemo_speech_asr_c.dll'))
    Check "the diarizer's closure" (Same $closure 'nemo_speech_asr_c.dll', 'nemo_speech_asr.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll', 'ggml-vulkan.dll', 'vcruntime140.dll', 'msvcp140.dll', 'msvcp140_1.dll', 'vcomp140.dll') ($closure -join ',')
    Check 'a root not in the package is skipped' (@(Get-LoadClosure (Package) @('absent.dll')).Count -eq 0)
} finally {
    Remove-Item -Recurse -Force $system32
}

if ($script:failures -gt 0) {
    Write-Output "$($script:failures) check(s) failed"
    exit 1
}
Write-Output 'all passed'
