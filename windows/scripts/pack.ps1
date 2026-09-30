#!/usr/bin/env pwsh
# The Windows release's files, from a published app (win-release-build.yml runs this; run it the
# same way on a PC, from anywhere): the installer, the update package and feed, and their SHA-256s.
# x64 only: 1.0 ships no ARM64 build (docs/RELEASING.md).
#
#   pwsh windows/scripts/pack.ps1 -Version 1.0.0 -AppDir <dotnet publish output> -OutDir <empty folder>
#
# First it checks the app is built for x64 (Inkwell.exe and the core), and what it needs from the
# PC it lands on (windows/scripts/lib/dll-imports.ps1): every DLL any of its binaries loads when it
# loads is beside Inkwell.exe or part of Windows, so ONNX Runtime must be the app's own, never the
# older onnxruntime.dll in Windows' System32; the Visual C++ runtime only as the copy beside
# Inkwell.exe, built for x64 (the engines' DLLs need it; build-core.ps1 put it with them; the core
# links the CRT statically, and the .NET and Windows App SDK binaries use the UCRT, which is part of
# Windows); and the Vulkan loader only where it may be missing (delay-loaded by the core, or behind
# the core's delay-loaded diarizer). It prints the Visual C++ runtime's version.
#
# Then Velopack's vpk (the version windows/.config/dotnet-tools.json pins) packs it:
# - Inkwell_X.Y.Z_x64-setup.exe, the installer: per user (no administrator), into
#   %LOCALAPPDATA%\InkwellApp, with a Start menu entry and an entry in Settings > Apps that
#   uninstalls it. The library (%LOCALAPPDATA%\Inkwell) is not in that folder, so an uninstall
#   leaves it. It refuses Windows older than 11 24H2 (10.0.26100), the app's floor. While it
#   installs it shows a splash with Microsoft's end-user terms ($SplashTerms below).
# - InkwellApp-X.Y.Z-full.nupkg and releases.win.json: the update and the feed the installed app
#   reads from the release (VelopackUpdater.cs). The feed names the package with its size and
#   SHA-256, which the app checks before installing it; this script checks the feed says the
#   truth about the package it wrote.
# - Inkwell_X.Y.Z_windows-sha256.txt: the three files' SHA-256s, in sha256sum's format, for people
#   to check a download against (the homepage says how).
#
# The package id, InkwellApp, names the install folder and the update chain: changing it later
# makes a different app to Velopack (no update path from the old one), so it stays. So does the
# channel, win: a new channel name would leave every install on the old one without updates.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Version,
    [Parameter(Mandatory)] [string]$AppDir,
    [Parameter(Mandatory)] [string]$OutDir
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PackId = 'InkwellApp'
# The Windows floor: 11 24H2 (Inkwell.csproj's TargetPlatformMinVersion), x64.
$Runtime = 'win10.0.26100-x64'
# The PE machine x64's binaries are built for.
$Machine = 0x8664

$windows = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib/dll-imports.ps1')

# The end-user terms the Windows App SDK's licence asks for (its section 3(b)(ii)), and the Windows
# SDK's for its .NET projection (its Distribution Requirements), on the installer's splash:
# Velopack's Setup has no text page, only an image shown while it installs, so the terms are on
# screen before Inkwell first runs, wherever the installer came from; the first run then asks the
# user to agree. The same terms are in Settings > About and the first run's step
# (Notices.WindowsAppSdkTerms), the release notes (win-release.yml) and the homepage
# (windows/HOMEPAGE-INSTALL.md): keep the four in step. The non-breaking spaces
# keep "Settings > About" on one line.
$SplashTerms = "Inkwell is free software under the MIT licence. It includes the runtime of Microsoft's " +
    "Windows App SDK and the Windows SDK's .NET projection, which Microsoft licenses separately under " +
    "the Microsoft Software License Terms, " +
    "shown in full in Inkwell's Settings`u{00A0}>`u{00A0}About. By installing or using Inkwell, you " +
    "agree to those terms for those components."

function Fail([string]$message) {
    Write-Output "::error title=pack::$message"
    throw "pack: $message"
}

# The splash, drawn here so its text is reviewed as text: a white PNG with the title and the terms.
function Write-Splash([string]$Path) {
    Add-Type -AssemblyName System.Drawing
    # Pixels; the bottom strip is left for Setup's progress bar.
    $width = 600; $height = 270; $margin = 32; $progress = 36
    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $titleFont = [System.Drawing.Font]::new('Segoe UI Semibold', 24, [System.Drawing.GraphicsUnit]::Pixel)
    $bodyFont = [System.Drawing.Font]::new('Segoe UI', 16, [System.Drawing.GraphicsUnit]::Pixel)
    $ink = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(0x1f, 0x1f, 0x1f))
    try {
        # A missing font falls back silently to another face: fail instead.
        foreach ($font in $titleFont, $bodyFont) {
            if (-not $font.Name.StartsWith('Segoe UI')) { Fail "the splash's font is $($font.Name), not Segoe UI" }
        }
        $graphics.Clear([System.Drawing.Color]::White)
        $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $graphics.DrawString('Installing Inkwell', $titleFont, $ink, [System.Drawing.PointF]::new($margin, $margin))
        $top = $margin + $titleFont.GetHeight($graphics) + 14
        $box = [System.Drawing.SizeF]::new($width - 2 * $margin, $height - $top - $progress)
        # The terms fit whole, or the pack fails: never cut off on screen.
        $needed = $graphics.MeasureString($SplashTerms, $bodyFont, [int]$box.Width)
        if ($needed.Height -gt $box.Height) { Fail "the splash's terms need $([int]$needed.Height) px of $([int]$box.Height)" }
        $graphics.DrawString($SplashTerms, $bodyFont, $ink, [System.Drawing.RectangleF]::new($margin, $top, $box.Width, $box.Height))
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $ink.Dispose(); $bodyFont.Dispose(); $titleFont.Dispose(); $graphics.Dispose(); $bitmap.Dispose()
    }
}

if ($Version -notmatch '^(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})$') { Fail "the version is X.Y.Z, not '$Version'" }
$AppDir = (Resolve-Path $AppDir).Path
foreach ($file in 'Inkwell.exe', 'ink_ffi.dll') {
    if (-not (Test-Path (Join-Path $AppDir $file))) { Fail "$file is not in $AppDir" }
}
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
if (@(Get-ChildItem $OutDir -Force).Count -gt 0) { Fail "$OutDir is not empty" }

# The app is built for x64: the release's one architecture.
foreach ($file in 'Inkwell.exe', 'ink_ffi.dll') {
    $found = Get-PeMachine (Join-Path $AppDir $file)
    if ($found -ne $Machine) { Fail ("{0} is machine {1}, not x64 (0x{2:X4})" -f $file, $(if ($null -eq $found) { '(unread)' } else { '0x{0:X4}' -f $found }), $Machine) }
}

# Visual Studio's dumpbin, found as build-core.ps1 finds the developer environment.
$dumpbin = (Get-Command dumpbin.exe -ErrorAction SilentlyContinue)?.Source
if (-not $dumpbin) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $dumpbin = Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC') -Recurse -Filter dumpbin.exe |
        Where-Object { $_.FullName -match '\\bin\\Hostx64\\x64\\' } | Select-Object -First 1 -ExpandProperty FullName
    if (-not $dumpbin) { Fail 'no dumpbin.exe (Visual Studio C++ build tools)' }
}

# What every binary loads, against what is beside Inkwell.exe and what Windows has
# (windows/scripts/lib/dll-imports.ps1). Inkwell.exe loads the core as it starts.
$imports = @{}
$binaries = @(Get-ChildItem $AppDir -Recurse -File -Include *.exe, *.dll)
foreach ($binary in $binaries) {
    $imports[$binary.Name.ToLowerInvariant()] = Read-DllImports $dumpbin $binary.FullName
}
$beside = @(Get-ChildItem $AppDir -File | ForEach-Object { $_.Name.ToLowerInvariant() })
$machines = @{}
foreach ($binary in Get-ChildItem $AppDir -File | Where-Object { $_.Extension -in '.exe', '.dll' }) {
    $machines[$binary.Name.ToLowerInvariant()] = Get-PeMachine $binary.FullName
}
$problems = @(Find-ImportProblems -Imports $imports -Beside $beside -Machines $machines -Machine $Machine -Roots 'inkwell.exe', 'ink_ffi.dll' -Core 'ink_ffi.dll')
if ($problems.Count -gt 0) {
    $problems | Sort-Object -Unique | ForEach-Object { Write-Output "::error title=pack::$_" }
    Fail "the app needs what a PC may not have ($($problems.Count) problem(s), above)"
}
# (Not $runtime: PowerShell's names ignore case, and $Runtime is vpk's.)
$vcRuntime = Get-VcRuntimeVersion $AppDir
Write-Output "pack: $($binaries.Count) binaries for x64; everything they load is beside Inkwell.exe or part of Windows"
Write-Output "pack: the Visual C++ runtime $($vcRuntime.Version) beside Inkwell.exe: $($vcRuntime.Files -join ', ')"

# vpk, as pinned. --skip-updates: it would otherwise ask NuGet for a newer vpk.
Push-Location $windows
try {
    dotnet tool restore
    if ($LASTEXITCODE -ne 0) { Fail 'dotnet tool restore failed' }
    $work = Join-Path $OutDir 'vpk'
    $splashDir = Join-Path $OutDir 'splash'
    New-Item -ItemType Directory $splashDir | Out-Null
    $splash = Join-Path $splashDir 'splash.png'
    Write-Splash $splash
    dotnet vpk pack --skip-updates --yes `
        --packId $PackId --packVersion $Version --packDir $AppDir --mainExe Inkwell.exe `
        --packTitle Inkwell --packAuthors Inkwell --icon (Join-Path $windows 'Inkwell\Assets\Inkwell.ico') `
        --splashImage $splash `
        --runtime $Runtime --channel win --shortcuts StartMenuRoot --noPortable --delta None `
        --outputDir $work
    if ($LASTEXITCODE -ne 0) { Fail "vpk pack failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

# What vpk wrote, and the feed checked against it.
$setupFrom = Join-Path $work "$PackId-win-Setup.exe"
$package = Join-Path $work "$PackId-$Version-full.nupkg"
$feedPath = Join-Path $work 'releases.win.json'
foreach ($file in $setupFrom, $package, $feedPath) {
    if (-not (Test-Path $file)) { Fail "vpk did not write $(Split-Path -Leaf $file) (it wrote: $((Get-ChildItem $work).Name -join ', '))" }
}
$feed = Get-Content $feedPath -Raw | ConvertFrom-Json
$assets = @($feed.Assets)
if ($assets.Count -ne 1) { Fail "releases.win.json lists $($assets.Count) packages, not the one full package" }
$asset = $assets[0]
$packageHash = (Get-FileHash -Algorithm SHA256 $package).Hash
$packageSize = (Get-Item $package).Length
if ($asset.PackageId -ne $PackId -or $asset.Version -ne $Version -or $asset.Type -ne 'Full' -or $asset.FileName -ne (Split-Path -Leaf $package)) {
    Fail "releases.win.json names $($asset.PackageId) $($asset.Version) $($asset.Type) $($asset.FileName), not $PackId $Version Full $(Split-Path -Leaf $package)"
}
if ([string]::IsNullOrEmpty($asset.SHA256) -or $asset.SHA256 -ine $packageHash -or [long]$asset.Size -ne $packageSize) {
    Fail "releases.win.json's size or SHA-256 for the package is not the package's"
}

# The release's files, under the names the release carries.
$setup = Join-Path $OutDir "Inkwell_${Version}_x64-setup.exe"
Move-Item $setupFrom $setup
Move-Item $package $OutDir
Move-Item $feedPath $OutDir
Remove-Item -Recurse -Force $work, $splashDir
$sums = Join-Path $OutDir "Inkwell_${Version}_windows-sha256.txt"
$lines = foreach ($file in Get-ChildItem $OutDir -File | Sort-Object Name) {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 $file.FullName).Hash.ToLowerInvariant(), $file.Name
}
# LF line ends, so sha256sum -c reads it anywhere.
[System.IO.File]::WriteAllText($sums, ($lines -join "`n") + "`n")
Get-Content $sums
Write-Output "pack: $((Get-ChildItem $OutDir -File).Count) files in $OutDir"
