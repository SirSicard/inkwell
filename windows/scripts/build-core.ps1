#!/usr/bin/env pwsh
# The core's DLL as the Windows release ships it (win-release.yml runs this; run it the same way on
# a PC, from anywhere): ink_ffi.dll with the Windows engines, Qwen3-ASR on llama.cpp with Vulkan
# (the CPU where the PC has no Vulkan GPU) and Silero VAD, in core/target/release/. Then it checks
# what the DLL needs from the PC it lands on:
#
# - The Vulkan loader, vulkan-1.dll, only delay-loaded, and only for vkGetInstanceProcAddr: the DLL
#   then loads on a PC without a Vulkan driver, and llama.cpp runs on the CPU there (ink-engines'
#   src/llama/no_vulkan.rs answers that one import when the loader is missing).
# - No Visual C++ runtime: the CRT is linked in statically (crt-static for Rust, LLAMA_STATIC_CRT
#   for llama.cpp's CMake build), so nothing like vcruntime140.dll, msvcp140.dll or the UCRT's
#   api-ms-win-crt-* sets has to be installed. Every other DLL it loads must be in System32.
# - That case run: tests/vulkan_missing.rs, linked the same way, in a process that cannot find the
#   loader.
#
# Needs: the Rust toolchain core/rust-toolchain.toml pins, the Vulkan SDK (VULKAN_SDK), LLVM's
# libclang (LIBCLANG_PATH; C:\Program Files\LLVM\bin when unset), CMake, Ninja, and Visual Studio's
# C++ build tools, whose developer environment it enters when cl.exe is not on PATH. The engine
# features are fixed here: the release is this build, nothing else.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The Windows release's engines (NeMo-Speech.cpp, the diarizer, is not in the Windows build yet).
$Features = 'engine-llama,ink-engines/engine-llama-vulkan,ink-engines/engine-silero'
# What the DLL may import from the Vulkan loader. vkGetInstanceProcAddr is ggml's first Vulkan call,
# which no_vulkan.rs answers when the loader is missing (Vulkan then fails to start, and ggml stops
# there). The other three take a device or a physical device, which exist only once vkCreateInstance
# has succeeded through a real loader. A name not listed fails the build until someone has checked
# it the same way: one called before an instance exists would end the process on a PC without Vulkan.
$VulkanImports = @('vkGetInstanceProcAddr', 'vkGetDeviceProcAddr', 'vkGetPhysicalDeviceFeatures2', 'vkCmdCopyBuffer')

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$core = Join-Path $root 'core'

function Fail([string]$message) {
    Write-Output "::error title=build-core::$message"
    throw "build-core: $message"
}

# Visual Studio's developer environment (cl.exe, link.exe, dumpbin.exe, delayimp.lib), entered in
# this process when it is not already.
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { Fail "no cl.exe on PATH and no vswhere.exe: install Visual Studio's C++ build tools" }
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { Fail 'vswhere found no Visual Studio with the x64 C++ tools' }
    Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
    Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null
}
foreach ($tool in 'cl.exe', 'link.exe', 'dumpbin.exe', 'cmake.exe', 'ninja.exe', 'cargo.exe') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Fail "$tool is not on PATH" }
}
if (-not $env:VULKAN_SDK -or -not (Test-Path (Join-Path $env:VULKAN_SDK 'Lib\vulkan-1.lib'))) {
    Fail 'VULKAN_SDK does not name a Vulkan SDK (Lib\vulkan-1.lib)'
}
if (-not $env:LIBCLANG_PATH) { $env:LIBCLANG_PATH = 'C:\Program Files\LLVM\bin' }
if (-not (Test-Path (Join-Path $env:LIBCLANG_PATH 'libclang.dll'))) { Fail "no libclang.dll in LIBCLANG_PATH ($env:LIBCLANG_PATH)" }

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

$dll = Join-Path $core 'target\release\ink_ffi.dll'
if (-not (Test-Path $dll)) { Fail "no $dll" }

# What the DLL loads when it loads, and what it loads later.
$dependents = @(dumpbin /nologo /dependents $dll)
if ($LASTEXITCODE -ne 0) { Fail 'dumpbin /dependents failed' }
$now = [System.Collections.Generic.List[string]]::new()
$later = [System.Collections.Generic.List[string]]::new()
$list = $null
foreach ($line in $dependents) {
    if ($line -match 'Image has the following dependencies') { $list = $now; continue }
    if ($line -match 'Image has the following delay load dependencies') { $list = $later; continue }
    if ($line -match '^\s*Summary') { $list = $null; continue }
    if ($null -ne $list -and $line -match '^\s+(\S+\.dll)\s*$') { $list.Add($Matches[1].ToLowerInvariant()) }
}
Write-Output "ink_ffi.dll loads: $(($now | Select-Object -Unique) -join ', ')"
Write-Output "ink_ffi.dll delay-loads: $($later -join ', ')"
if ($now.Count -eq 0) { Fail 'dumpbin listed no dependencies: its output was not read' }

$problems = [System.Collections.Generic.List[string]]::new()
foreach ($name in $now) {
    if ($name -eq 'vulkan-1.dll') { $problems.Add('vulkan-1.dll is loaded at load time, not delay-loaded'); continue }
    if ($name -match '^(vcruntime|msvcp|concrt|vcomp|ucrtbase)' -or $name -match '^api-ms-win-crt-') {
        $problems.Add("$name is a Visual C++ runtime DLL: the CRT is not linked statically")
        continue
    }
    # API sets (api-ms-win-core-*) are the loader's own; everything else must ship with Windows.
    if ($name -notmatch '^(api|ext)-ms-win-' -and -not (Test-Path (Join-Path $env:SystemRoot "System32\$name"))) {
        $problems.Add("$name is not a Windows DLL (not in System32)")
    }
}
foreach ($name in $later) {
    if ($name -ne 'vulkan-1.dll') { $problems.Add("$name is delay-loaded: only vulkan-1.dll is expected") }
}
if (-not $later.Contains('vulkan-1.dll')) { $problems.Add('vulkan-1.dll is not delay-loaded') }

# The functions imported from the Vulkan loader: its section of dumpbin /imports, up to the next
# DLL's.
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

if ($problems.Count -gt 0) {
    $problems | ForEach-Object { Write-Output "::error title=build-core::$_" }
    throw "build-core: ink_ffi.dll needs what a PC may not have ($($problems.Count) problem(s), above)"
}
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

$hash = (Get-FileHash -Algorithm SHA256 $dll).Hash.ToLowerInvariant()
Write-Output "build-core: ink_ffi.dll ($Features), static CRT, Vulkan delay-loaded; sha256 $hash"
