#!/usr/bin/env pwsh
# The core as the Windows release ships it (win-release-build.yml runs this; run it the same way on
# a PC, from anywhere): ink_ffi.dll with the Windows engines, built for the PC's own architecture
# (x64, or ARM64 on an ARM64 PC: never cross-compiled), in core/target/release/ (CARGO_TARGET_DIR's
# release/ when that is set); then the DLL, the engines' DLLs and the Visual C++ runtime DLLs these
# need together in its inkwell-core/, the folder the app is published with (InkCoreDir:
# Inkwell.csproj puts every DLL in it beside Inkwell.exe).
#
# The engines, by architecture ($ReleaseFeatures below):
# - x64, the release: Qwen3-ASR on llama.cpp with Vulkan (the CPU where the PC has no Vulkan GPU),
#   Silero VAD, Parakeet on sherpa-onnx, and the NeMo-Speech.cpp diarizer.
# - ARM64, not released (1.0 ships x64 alone; docs/RELEASING.md): Qwen3-ASR on llama.cpp on the
#   CPU, Silero VAD and Parakeet on sherpa-onnx; no Vulkan, and no diarizer.
#
# Then it checks what they need from the PC they land on:
# - No C runtime for the core: its CRT is linked in statically (crt-static for Rust,
#   LLAMA_STATIC_CRT for llama.cpp's CMake build), so the DLL loads neither the Visual C++ runtime
#   (vcruntime140.dll, msvcp140.dll) nor the UCRT (ucrtbase.dll, the api-ms-win-crt-* sets).
# - x64: the Vulkan loader, vulkan-1.dll, only delay-loaded, and only for the functions listed
#   below: the DLL then loads on a PC without a Vulkan driver, and llama.cpp runs on the CPU there
#   (ink-engines' src/llama/no_vulkan.rs answers vkGetInstanceProcAddr when the loader is missing).
#   That case is run too: tests/vulkan_missing.rs, linked the same way, in a process that cannot
#   find the loader.
# - The diarizer's DLL, nemo_speech_asr_c.dll, only delay-loaded as well: its Vulkan backend loads
#   the Vulkan loader as soon as it loads, so a core that loaded it at once would not start without
#   a Vulkan driver (ink-engines' src/nemo.rs checks it loads before its first call).
# - Everything the core and the engines' DLLs load when they load is beside them in inkwell-core/
#   or part of Windows (windows/scripts/lib/dll-imports.ps1): ONNX Runtime must be the one beside
#   them, never Windows' own older onnxruntime.dll in System32, and the Visual C++ runtime the
#   copy beside them, of this architecture.
#
# The Visual C++ runtime: sherpa-onnx's archive and NeMo-Speech.cpp's build link it dynamically
# (/MD), so the DLLs of it that the engines' DLLs import (and those these import in turn), and no
# others, are copied beside them from Visual Studio's redistributable folder for this architecture
# (VCToolsRedistDir, set by the developer environment: $VcRedist's CRT and OpenMP folders, which
# the Visual Studio licence lets an app redistribute; never the build machine's System32). Their
# version is printed, and win-release-build.yml records it in the release.
#
# Needs: the Rust toolchain core/rust-toolchain.toml pins; CMake and Ninja; Visual Studio 2026's C++
# build tools for this architecture, whose developer environment it enters when cl.exe is not on
# PATH (its redistributable folder, VCToolsRedistDir, holds the runtime copied above); LLVM
# (libclang for llama.cpp's bindings, LIBCLANG_PATH, LLVM's bin under Program Files when
# unset; on ARM64 also clang-cl, since ggml refuses MSVC on ARM); on x64 the Vulkan SDK
# (VULKAN_SDK); SHERPA_ONNX_DIR, the unpacked sherpa-onnx-v1.13.4-win-<x64|arm64>-shared-MD-Release-
# no-tts-lib archive, whose files ink-engines' build.rs checks against its pins; and, where the
# features name the diarizer, NEMO_SPEECH_DIR, the prefix core/crates/ink-engines/native/
# build-nemo-speech.sh installed, which build.rs checks against its manifest. The features are
# fixed here: the release is this build, nothing else.
#
#   pwsh windows/scripts/build-core.ps1                 build and check
#   pwsh windows/scripts/build-core.ps1 -ListFeatures   print this architecture's features, and stop
[CmdletBinding()]
param([switch]$ListFeatures)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The engines each architecture's release ships, as ink-ffi's cargo features. ink-notices holds its
# WINDOWS_RELEASE_FEATURES (x64) and WINDOWS_ARM64_RELEASE_FEATURES to these lines (a test), and
# win-release-build.yml builds the diarizer's prefix for an architecture whose line names it.
$ReleaseFeatures = @{
    X64 = 'engine-llama,ink-engines/engine-llama-vulkan,ink-engines/engine-silero,ink-engines/engine-sherpa,ink-engines/engine-nemo'
    # ARM64 is not released in 1.0 (docs/RELEASING.md): this is what an ARM64 PC builds, without
    # the diarizer (build-nemo-speech.sh builds upstream's Vulkan preset, and there is no ARM64
    # Vulkan SDK step). ink-notices' WINDOWS_ARM64_RELEASE_FEATURES must match (its test fails
    # until it does).
    Arm64 = 'engine-llama,ink-engines/engine-silero,ink-engines/engine-sherpa'
}
# What the DLL may import from the Vulkan loader. vkGetInstanceProcAddr is ggml's first Vulkan call,
# which no_vulkan.rs answers when the loader is missing (Vulkan then fails to start, and ggml stops
# there). The other three take a device or a physical device, which exist only once vkCreateInstance
# has succeeded through a real loader. A name not listed fails the build until someone has checked
# it the same way: one called before an instance exists would end the process on a PC without Vulkan.
$VulkanImports = @('vkGetInstanceProcAddr', 'vkGetDeviceProcAddr', 'vkGetPhysicalDeviceFeatures2', 'vkCmdCopyBuffer')
# sherpa-onnx's DLLs the app ships: the C API, the ONNX Runtime it loads, and the provider bridge
# ONNX Runtime loads (build.rs pins all three).
$SherpaDlls = @('sherpa-onnx-c-api.dll', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll')
# The Visual C++ runtime the release ships (Microsoft.VC145.CRT and .OpenMP): Visual Studio 2026's,
# under the licence terms About shows for it and the first run asks the user to agree to
# (windows/Inkwell.Core/Screens/About/Notices.cs, vc-runtime). Another redistributable would need
# its own terms there first, so any other stops the build.
$VcRedist = 'VC145'

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$core = Join-Path $root 'core'
. (Join-Path $PSScriptRoot 'lib/dll-imports.ps1')

function Fail([string]$message) {
    Write-Output "::error title=build-core::$message"
    throw "build-core: $message"
}

# The PC's own architecture, not the one this PowerShell may be emulated as.
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
if (-not $ReleaseFeatures.ContainsKey($arch)) { Fail "this PC is ${arch}: the Windows release is built on x64 and on ARM64" }
$Features = $ReleaseFeatures[$arch]
if ($ListFeatures) {
    Write-Output $Features
    return
}
$vulkan = $Features -match 'engine-llama-vulkan'
$nemo = $Features -match 'engine-nemo'
$sherpa = $Features -match 'engine-sherpa'

# Visual Studio's developer environment for this architecture (cl.exe, link.exe, dumpbin.exe,
# delayimp.lib), entered in this process when it is not already.
$vsArch = if ($arch -eq 'Arm64') { 'arm64' } else { 'x64' }
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { Fail "no cl.exe on PATH and no vswhere.exe: install Visual Studio's C++ build tools" }
    $component = if ($arch -eq 'Arm64') { 'Microsoft.VisualStudio.Component.VC.Tools.ARM64' } else { 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' }
    $vs = & $vswhere -latest -products * -requires $component -property installationPath
    if (-not $vs) { Fail "vswhere found no Visual Studio with the $vsArch C++ tools" }
    Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
    Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments "-arch=$vsArch -host_arch=$vsArch" | Out-Null
}
# LLVM: libclang for llama.cpp's bindings, and on ARM64 clang-cl, first on PATH, for llama.cpp's
# C and C++ (ggml refuses MSVC on ARM: "MSVC is not supported for ARM, use clang").
$llvm = Join-Path $env:ProgramW6432 'LLVM\bin'
if (-not $env:LIBCLANG_PATH) { $env:LIBCLANG_PATH = $llvm }
if (-not (Test-Path (Join-Path $env:LIBCLANG_PATH 'libclang.dll'))) { Fail "no libclang.dll in LIBCLANG_PATH ($env:LIBCLANG_PATH)" }
$tools = @('cl.exe', 'link.exe', 'dumpbin.exe', 'cmake.exe', 'ninja.exe', 'cargo.exe', 'rustc.exe')
if ($arch -eq 'Arm64') {
    if (-not (Get-Command clang-cl.exe -ErrorAction SilentlyContinue)) { $env:Path = "$llvm;$env:Path" }
    $env:CMAKE_C_COMPILER = 'clang-cl'
    $env:CMAKE_CXX_COMPILER = 'clang-cl'
    # C++ exceptions: cmake-rs gives CMake cc's flags as CMAKE_CXX_FLAGS, which replace CMake's own
    # /EHsc, and clang-cl without it refuses ggml's try and throw ("cannot use 'try' with
    # exceptions disabled"; cl only warns). cc adds CXXFLAGS to those flags. (Like a CMAKE_*
    # variable, a change here needs `cargo clean --release -p llama-cpp-sys-2` locally.)
    $env:CXXFLAGS = '/EHsc'
    $tools += 'clang-cl.exe'
}
foreach ($tool in $tools) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Fail "$tool is not on PATH" }
}
if ($vulkan -and (-not $env:VULKAN_SDK -or -not (Test-Path (Join-Path $env:VULKAN_SDK 'Lib\vulkan-1.lib')))) {
    Fail 'VULKAN_SDK does not name a Vulkan SDK (Lib\vulkan-1.lib)'
}
if ($sherpa -and -not $env:SHERPA_ONNX_DIR) { Fail "SHERPA_ONNX_DIR is not set: the unpacked sherpa-onnx-v1.13.4-win-$vsArch-shared-MD-Release-no-tts-lib archive" }
if ($nemo -and -not $env:NEMO_SPEECH_DIR) { Fail 'NEMO_SPEECH_DIR is not set: the prefix core/crates/ink-engines/native/build-nemo-speech.sh installed' }
if (-not $nemo -and $env:NEMO_SPEECH_DIR) { Fail "NEMO_SPEECH_DIR is set, but the $arch release ships no diarizer (the switch in this script)" }

# The toolchain builds for this architecture, natively: a toolchain whose host is x64 on an ARM64
# PC would build an x64 core, which would run only under emulation.
$triple = if ($arch -eq 'Arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
Push-Location $core
try {
    $rustHost = @(rustc -vV) | Where-Object { $_ -match '^host: ' } | ForEach-Object { $_.Substring(6).Trim() }
} finally {
    Pop-Location
}
if ($rustHost -ne $triple) { Fail "rustc's host is '$rustHost', not $triple" }

# Ninja: with Visual Studio's generator, MSBuild runs ggml's shader generator's install step before
# its configure step (ink-engines' Cargo.toml).
$env:CMAKE_GENERATOR = 'Ninja'
# The CRT, statically: Rust's side (every crate; the cc builds read it) and llama.cpp's CMake build.
# There, LLAMA_STATIC_CRT puts /MT in the flags, but CMake's own runtime setting (policy CMP0091)
# still adds /MD after it: CMAKE_MSVC_RUNTIME_LIBRARY, which llama-cpp-sys-2 passes to CMake like
# every CMAKE_* variable, sets it. Mixed, the link fails on the CRT's missing __imp_ symbols. (Its
# build script does not rebuild when a CMAKE_* variable changes: after changing one locally, run
# `cargo clean --release -p llama-cpp-sys-2` first.)
$env:RUSTFLAGS = '-C target-feature=+crt-static'
$env:LLAMA_STATIC_CRT = '1'
$env:CMAKE_MSVC_RUNTIME_LIBRARY = 'MultiThreaded'

Push-Location $core
try {
    cargo build --release --locked -p ink-ffi --lib --features $Features
    if ($LASTEXITCODE -ne 0) { Fail "cargo build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

# Where cargo built it: CARGO_TARGET_DIR when set (on a PC whose checkout path is long, a short one
# keeps llama.cpp's deepest build paths under Windows' 260 characters), else core\target.
$target = if ($env:CARGO_TARGET_DIR) { [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR, $core) } else { Join-Path $core 'target' }
$dll = Join-Path $target 'release\ink_ffi.dll'
if (-not (Test-Path $dll)) { Fail "no $dll" }

# The core as it ships: the DLL and the engines' DLLs, in a folder of their own that holds nothing
# else (Inkwell.csproj copies every DLL in it).
$out = Join-Path $target 'release\inkwell-core'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory $out | Out-Null
Copy-Item $dll $out
if ($sherpa) {
    # From the directory ink-engines' build.rs checked against its pins in this very build (it runs
    # again whenever one of them changes).
    foreach ($name in $SherpaDlls) {
        $from = Join-Path $env:SHERPA_ONNX_DIR "lib\$name"
        if (-not (Test-Path $from)) { Fail "SHERPA_ONNX_DIR has no lib\$name" }
        Copy-Item $from $out
    }
}
if ($nemo) {
    # Every DLL the prefix's manifest lists (NeMo's own and its ggml's), each checked against the
    # manifest again as it is copied.
    $manifest = Join-Path $env:NEMO_SPEECH_DIR 'share\inkwell\nemo-speech.manifest'
    if (-not (Test-Path $manifest)) { Fail "NEMO_SPEECH_DIR has no share\inkwell\nemo-speech.manifest" }
    $copied = 0
    foreach ($line in Get-Content $manifest) {
        if ($line -notmatch '^sha256 ([0-9a-f]{64}) bin/([^/\\]+\.dll)$') { continue }
        $sum = $Matches[1]
        $copy = Join-Path $out $Matches[2]
        Copy-Item (Join-Path $env:NEMO_SPEECH_DIR "bin\$($Matches[2])") $copy
        if ((Get-FileHash -Algorithm SHA256 $copy).Hash -ne $sum) { Fail "$($Matches[2]) is not the file nemo-speech.manifest lists" }
        $copied++
    }
    if ($copied -eq 0) { Fail 'nemo-speech.manifest lists no DLL' }
}

# The Visual C++ runtime DLLs the folder's DLLs import (above), from the redistributable folder.
$dumpbin = (Get-Command dumpbin.exe).Source
$redistFolders = if ($env:VCToolsRedistDir) { @('CRT', 'OpenMP') | ForEach-Object { Join-Path $env:VCToolsRedistDir "$vsArch\Microsoft.$VcRedist.$_" } } else { @() }
$pending = [System.Collections.Generic.Queue[string]]::new()
foreach ($file in Get-ChildItem $out -File -Filter *.dll) { $pending.Enqueue($file.FullName) }
$runtimeCopied = 0
while ($pending.Count -gt 0) {
    $imports = Read-DllImports $dumpbin $pending.Dequeue()
    foreach ($name in @($imports.Load) + @($imports.Delay) | Where-Object { Test-VcRuntimeDll $_ }) {
        $copy = Join-Path $out $name
        if (Test-Path $copy) { continue }
        if (-not $env:VCToolsRedistDir) { Fail "the engines need $name, and VCToolsRedistDir is not set: run this in Visual Studio's developer environment" }
        $from = @($redistFolders | ForEach-Object { Join-Path $_ $name } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })
        if ($from.Count -eq 0) { Fail "the engines need $name, which Visual Studio's redistributable folder does not hold for $vsArch ($($redistFolders -join ', '))" }
        Copy-Item -LiteralPath $from[0] $copy
        $runtimeCopied++
        $pending.Enqueue($copy)
    }
}
$runtime = $null
if ($runtimeCopied -gt 0) {
    $runtime = Get-VcRuntimeVersion $out
    Write-Output "the Visual C++ runtime $($runtime.Version), from $($env:VCToolsRedistDir) (Microsoft.$VcRedist, $vsArch): $($runtime.Files -join ', ')"
}

# What the DLL loads when it loads, and what it loads later.
$coreImports = Read-DllImports $dumpbin $dll
Write-Output "ink_ffi.dll loads: $($coreImports.Load -join ', ')"
Write-Output "ink_ffi.dll delay-loads: $($coreImports.Delay -join ', ')"
if ($coreImports.Load.Count -eq 0) { Fail 'dumpbin listed no dependencies: its output was not read' }

$problems = [System.Collections.Generic.List[string]]::new()
foreach ($name in $coreImports.Load) {
    if ($name -eq 'ucrtbase.dll' -or $name -match '^api-ms-win-crt-' -or (Test-VcRuntimeDll $name)) {
        $problems.Add("ink_ffi.dll loads $name, a C runtime DLL: the CRT is not linked statically")
    }
}
# Delay-loaded: exactly the Vulkan loader where llama.cpp has Vulkan, and the diarizer's DLL where
# the diarizer ships.
$delayed = @()
if ($vulkan) { $delayed += 'vulkan-1.dll' }
if ($nemo) { $delayed += 'nemo_speech_asr_c.dll' }
foreach ($name in $delayed) {
    if ($coreImports.Delay -notcontains $name) { $problems.Add("$name is not delay-loaded") }
}
foreach ($name in $coreImports.Delay) {
    if ($delayed -notcontains $name) { $problems.Add("$name is delay-loaded: only $(if ($delayed) { $delayed -join ' and ' } else { 'nothing' }) should be") }
}

if ($vulkan) {
    # The functions imported from the Vulkan loader: its section of dumpbin /imports, up to the
    # next DLL's.
    $imports = @(dumpbin /nologo /imports $dll)
    if ($LASTEXITCODE -ne 0) { Fail 'dumpbin /imports failed' }
    $inVulkan = $false
    $vulkanNames = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $imports) {
        if ($line -match '^\s{4}(\S+\.dll)\s*$') { $inVulkan = ($Matches[1] -ieq 'vulkan-1.dll'); continue }
        # An import: its IAT address, its hint, its name.
        if ($inVulkan -and $line -match '^\s+(?:[0-9A-Fa-f]+\s+)+(vk\w+)\s*$') { $vulkanNames.Add($Matches[1]) }
    }
    Write-Output "from vulkan-1.dll: $($vulkanNames -join ', ')"
    if ($vulkanNames.Count -eq 0) { $problems.Add('no function is imported from vulkan-1.dll: the imports were not read') }
    foreach ($name in $vulkanNames) {
        if ($VulkanImports -cnotcontains $name) {
            $problems.Add("$name is imported from vulkan-1.dll and not among those checked for a PC without Vulkan ($($VulkanImports -join ', '); see no_vulkan.rs)")
        }
    }
    if (-not $vulkanNames.Contains('vkGetInstanceProcAddr')) { $problems.Add('vkGetInstanceProcAddr is not imported from vulkan-1.dll: no_vulkan.rs answers a call that no longer comes') }

    # The delay-load helper's notify hook is no_vulkan.rs's (the DLL exports it, as it exports every
    # #[no_mangle] symbol): delayimp.lib's own, null, would leave a missing loader unanswered.
    $exports = @(dumpbin /nologo /exports $dll)
    if ($LASTEXITCODE -ne 0) { Fail 'dumpbin /exports failed' }
    if (-not ($exports -match '\s__pfnDliNotifyHook2\b')) { $problems.Add('the delay-load hook (__pfnDliNotifyHook2) is not the one in the core (no_vulkan.rs)') }
}

# Everything the core's folder loads: beside it, or part of Windows.
$folder = @{}
$machines = @{}
foreach ($file in Get-ChildItem $out -File -Filter *.dll) {
    $folder[$file.Name.ToLowerInvariant()] = Read-DllImports $dumpbin $file.FullName
    $machines[$file.Name.ToLowerInvariant()] = Get-PeMachine $file.FullName
}
$beside = @(Get-ChildItem $out -File | ForEach-Object { $_.Name.ToLowerInvariant() })
$machine = if ($arch -eq 'Arm64') { 0xAA64 } else { 0x8664 }
foreach ($problem in (Find-ImportProblems -Imports $folder -Beside $beside -Machines $machines -Machine $machine -Roots 'ink_ffi.dll' -Core 'ink_ffi.dll')) {
    $problems.Add($problem)
}

if ($problems.Count -gt 0) {
    $problems | Sort-Object -Unique | ForEach-Object { Write-Output "::error title=build-core::$_" }
    throw "build-core: the core needs what a PC may not have ($($problems.Count) problem(s), above)"
}
if ($vulkan) {
    # The case itself, in a process linked as the DLL is and unable to find the loader (ink-engines'
    # tests/vulkan_missing.rs): the backend starts and llama.cpp chooses the CPU. Same flags, so the
    # llama.cpp build above is reused.
    Push-Location $core
    try {
        cargo test --release --locked -p ink-engines --features engine-llama-vulkan --test vulkan_missing
        if ($LASTEXITCODE -ne 0) { Fail "the core does not start without Vulkan (tests/vulkan_missing.rs, $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

Write-Output "build-core: $arch, $Features; static CRT$(if ($vulkan) { ', Vulkan delay-loaded' })$(if ($nemo) { ', the diarizer delay-loaded' })$(if ($runtime) { "; the engines' Visual C++ runtime $($runtime.Version) beside them" })"
foreach ($file in Get-ChildItem $out -File | Sort-Object Name) {
    Write-Output ('{0}  {1}' -f (Get-FileHash -Algorithm SHA256 $file.FullName).Hash.ToLowerInvariant(), $file.Name)
}
