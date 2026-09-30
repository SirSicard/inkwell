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
    # The x64 release: sherpa-onnx at load time, NeMo delay-loaded, its Vulkan backend behind it.
    function Package {
        @{
            'inkwell.exe' = Deps @('kernel32.dll', 'api-ms-win-crt-heap-l1-1-0.dll')
            'ink_ffi.dll' = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll') @('vulkan-1.dll', 'nemo_speech_asr_c.dll')
            'sherpa-onnx-c-api.dll' = Deps @('onnxruntime.dll', 'kernel32.dll')
            'onnxruntime.dll' = Deps @('kernel32.dll', 'dxgi.dll', 'dbghelp.dll')
            'onnxruntime_providers_shared.dll' = Deps @('kernel32.dll')
            'nemo_speech_asr_c.dll' = Deps @('nemo_speech_asr.dll', 'kernel32.dll')
            'nemo_speech_asr.dll' = Deps @('ggml.dll', 'ggml-base.dll', 'kernel32.dll')
            'ggml.dll' = Deps @('ggml-cpu.dll', 'ggml-vulkan.dll', 'ggml-base.dll')
            'ggml-base.dll' = Deps @('kernel32.dll')
            'ggml-cpu.dll' = Deps @('ggml-base.dll', 'kernel32.dll')
            'ggml-vulkan.dll' = Deps @('ggml-base.dll', 'vulkan-1.dll', 'kernel32.dll')
            # The Windows App SDK delay-loads parts of Windows an edition may lack.
            'microsoft.ui.xaml.dll' = Deps @('kernel32.dll') @('ext-ms-win-something-l1-1-0.dll', 'not-on-this-edition.dll')
        }
    }
    function Problems([hashtable]$Imports, [string[]]$Beside = @($Imports.Keys)) {
        Find-ImportProblems -Imports $Imports -Beside $Beside -Roots 'inkwell.exe', 'ink_ffi.dll' -Core 'ink_ffi.dll' -IsWindowsDll $partOfWindows
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

    $p = Package
    $p['ggml-cpu.dll'] = Deps @('ggml-base.dll', 'vcomp140.dll') @()
    $p['sherpa-onnx-c-api.dll'] = Deps @('onnxruntime.dll') @('msvcp140.dll')
    $found = @(Problems $p @(@($p.Keys) + 'vcomp140.dll'))
    Check 'the Visual C++ runtime, even beside the app, even delay-loaded' ($found.Count -eq 2 -and ($found -join ';') -match 'ggml-cpu.dll needs vcomp140.dll, a Visual C\+\+ runtime DLL' -and ($found -join ';') -match 'sherpa-onnx-c-api.dll needs msvcp140.dll') ($found -join '; ')

    # ARM64 without the diarizer: no delay-load at all.
    $arm = @{
        'inkwell.exe' = Deps @('kernel32.dll')
        'ink_ffi.dll' = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll')
        'sherpa-onnx-c-api.dll' = Deps @('onnxruntime.dll')
        'onnxruntime.dll' = Deps @('kernel32.dll')
    }
    $found = @(Problems $arm)
    Check 'the ARM64 release without the diarizer' ($found.Count -eq 0) ($found -join '; ')
    $arm['ink_ffi.dll'] = Deps @('kernel32.dll', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll') @('vulkan-1.dll')
    Check "a delay-loaded Vulkan loader may be missing" (@(Problems $arm).Count -eq 0)

    Write-Output 'load closures'
    function Same([string[]]$A, [string[]]$B) { (($A | Sort-Object) -join ',') -eq (($B | Sort-Object) -join ',') }
    $closure = @(Get-LoadClosure (Package) @('ink_ffi.dll'))
    Check "the core's load-time closure stops at its delay-loads" (Same $closure 'ink_ffi.dll', 'onnxruntime.dll', 'sherpa-onnx-c-api.dll') ($closure -join ',')
    $closure = @(Get-LoadClosure (Package) @('nemo_speech_asr_c.dll'))
    Check "the diarizer's closure" (Same $closure 'nemo_speech_asr_c.dll', 'nemo_speech_asr.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll', 'ggml-vulkan.dll') ($closure -join ',')
    Check 'a root not in the package is skipped' (@(Get-LoadClosure (Package) @('absent.dll')).Count -eq 0)
} finally {
    Remove-Item -Recurse -Force $system32
}

if ($script:failures -gt 0) {
    Write-Output "$($script:failures) check(s) failed"
    exit 1
}
Write-Output 'all passed'
