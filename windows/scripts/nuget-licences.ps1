#!/usr/bin/env pwsh
# The NuGet licence check (CLAUDE.md, Licences): every package the Windows solution restores,
# direct or transitive, including the packs the SDK downloads for it, carries a licence on the
# allowlist, or is listed in nuget-licence-exceptions.json with the SHA-256 of its licence text and
# the reason it is accepted. win.yml runs it after `dotnet restore`; run it the same way locally,
# from windows/ (where global.json pins the SDK):
#
#   dotnet restore Inkwell.slnx
#   pwsh scripts/nuget-licences.ps1
#
# It reads what restore wrote (obj/project.assets.json) and each package's .nuspec from the NuGet
# packages folder, so it needs no network. It fails on a licence it cannot read, on an exception
# whose licence text changed (new terms need a new review), and on an exception nothing uses.
#
# An exception is {"id": "<package id>", "licence": "<what this script prints for it>",
# "reason": "<why it is accepted, and who accepted it when>"}; "licence" is "file <name>
# sha256:<hash of the text, LF line ends>" or "url <licenseUrl>".
[CmdletBinding()]
param(
    # The Windows solution's directory.
    [string]$Root = (Split-Path -Parent $PSScriptRoot),
    # SPDX ids accepted without an exception.
    [string[]]$Allow = @('MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'Zlib')
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$exceptionsPath = Join-Path $PSScriptRoot 'nuget-licence-exceptions.json'
$exceptions = @{}
foreach ($e in @(Get-Content $exceptionsPath -Raw | ConvertFrom-Json)) {
    if ($null -eq $e) { continue }
    $exceptions[$e.id.ToLowerInvariant()] = $e
}

# Every package restore resolved, from every project's assets file.
$packages = @{}
$assetsFiles = @(Get-ChildItem -Path $Root -Recurse -Filter project.assets.json |
    Where-Object { $_.Directory.Name -eq 'obj' })
if ($assetsFiles.Count -eq 0) {
    throw "no obj/project.assets.json under ${Root}: run dotnet restore first"
}
foreach ($file in $assetsFiles) {
    $assets = Get-Content $file.FullName -Raw | ConvertFrom-Json -AsHashtable
    $folders = @($assets['packageFolders'].Keys)
    foreach ($key in $assets['libraries'].Keys) {
        if ($assets['libraries'][$key]['type'] -ne 'package') { continue }
        $id, $version = $key -split '/', 2
        $packages["$($id.ToLowerInvariant())/$($version.ToLowerInvariant())"] = @{ Id = $id; Version = $version; Folders = $folders }
    }
    # Packs the SDK downloads for the project (runtime packs, the ReadyToRun compiler, the Windows
    # SDK projection): not libraries, but they ship or build the app all the same.
    foreach ($tfm in $assets['project']['frameworks'].Keys) {
        foreach ($dep in @($assets['project']['frameworks'][$tfm]['downloadDependencies'])) {
            if ($null -eq $dep) { continue }
            # An exact range, "[10.0.12, 10.0.12]": each bound names the same version.
            foreach ($v in @($dep['version'] -replace '[\[\]\(\) ]', '' -split ',' | Where-Object { $_ } | Sort-Object -Unique)) {
                $packages["$($dep['name'].ToLowerInvariant())/$($v.ToLowerInvariant())"] = @{ Id = $dep['name']; Version = $v; Folders = $folders }
            }
        }
    }
}

function Test-Expression([string]$expression) {
    # SPDX: any OR branch whose AND terms are all allowed, compared case-sensitively. WITH (an
    # exception clause) and parentheses (grouping this reading does not follow) need a person.
    if ($expression -cmatch '\bWITH\b' -or $expression.Contains('(') -or $expression.Contains(')')) { return $false }
    foreach ($branch in ($expression -csplit '\s+OR\s+')) {
        $terms = @($branch -csplit '\s+AND\s+' | ForEach-Object { $_.Trim() })
        if (@($terms | Where-Object { $Allow -cnotcontains $_ }).Count -eq 0) { return $true }
    }
    return $false
}

$failures = [System.Collections.Generic.List[string]]::new()
$used = @{}
$rows = foreach ($key in ($packages.Keys | Sort-Object)) {
    $p = $packages[$key]
    $lowerId = $p.Id.ToLowerInvariant()
    $dir = $null
    foreach ($folder in $p.Folders) {
        $candidate = Join-Path $folder (Join-Path $lowerId $p.Version.ToLowerInvariant())
        if (Test-Path $candidate) { $dir = $candidate; break }
    }
    if ($null -eq $dir) {
        $failures.Add("$($p.Id) $($p.Version): not in the packages folder (restore first)")
        continue
    }
    [xml]$nuspec = Get-Content (Join-Path $dir "$lowerId.nuspec") -Raw
    $meta = $nuspec.package.metadata
    $licence = $meta.SelectSingleNode("*[local-name()='license']")
    $url = $meta.SelectSingleNode("*[local-name()='licenseUrl']")
    $verdict = $null
    $what = $null
    if ($null -ne $licence -and $licence.GetAttribute('type') -eq 'expression') {
        $what = $licence.InnerText.Trim()
        if (Test-Expression $what) { $verdict = 'allowed' }
    } elseif ($null -ne $licence -and $licence.GetAttribute('type') -eq 'file') {
        $text = (Get-Content (Join-Path $dir $licence.InnerText.Trim()) -Raw) -replace "`r`n", "`n"
        $sha = [System.Convert]::ToHexString(
            [System.Security.Cryptography.SHA256]::HashData([System.Text.Encoding]::UTF8.GetBytes($text))).ToLowerInvariant()
        $what = "file $($licence.InnerText.Trim()) sha256:$sha"
    } elseif ($null -ne $url) {
        $what = "url $($url.InnerText.Trim())"
    } else {
        $what = 'none'
    }
    if ($null -eq $verdict -and $exceptions.ContainsKey($lowerId)) {
        $e = $exceptions[$lowerId]
        if ($what -ceq $e.licence) {
            $verdict = "exception: $($e.reason)"
            $used[$lowerId] = $true
        } else {
            $failures.Add("$($p.Id) $($p.Version): its licence is now '$what', the exception reviewed '$($e.licence)'")
        }
    }
    if ($null -eq $verdict) {
        $verdict = 'REFUSED'
        $failures.Add("$($p.Id) $($p.Version): '$what' is not on the allowlist ($($Allow -join ', ')) and has no exception")
    }
    [pscustomobject]@{ Package = $p.Id; Version = $p.Version; Licence = $what; Verdict = $verdict }
}
$rows | Format-Table -AutoSize -Wrap | Out-String -Width 200 | Write-Output

foreach ($id in $exceptions.Keys) {
    if (-not $used.ContainsKey($id)) {
        $failures.Add("nuget-licence-exceptions.json: '$id' is used by no restored package: remove it")
    }
}
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Output "nuget-licences: $_" }
    exit 1
}
Write-Output "nuget-licences: $($packages.Count) packages, all allowed"
