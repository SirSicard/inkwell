// The notices of the Rust crates linked into the core (RustNotices.g.cs): About lists every one,
// and the list is the one generated from this checkout's Cargo.lock for the Windows target. The
// list itself comes from cargo's resolution of the Windows release build (`cargo run -p ink-ffi
// --bin ink-notices -- --windows`); `ink-notices --windows --check` compares the whole file with a
// fresh run. (The Mac's RustNoticesTests, for the Windows file.)
using System.Globalization;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class RustNoticesTests
{
    /// <summary>64-bit FNV-1a, as ink-notices computes the fingerprint (the constants are the published ones).</summary>
    internal static string Fnv1a(IEnumerable<byte> bytes)
    {
        var hash = 0xcbf2_9ce4_8422_2325UL;
        foreach (var b in bytes)
        {
            hash = unchecked((hash ^ b) * 0x0000_0100_0000_01b3UL);
        }
        return hash.ToString("x16", CultureInfo.InvariantCulture);
    }

    internal static string Fnv1a(string text) => Fnv1a(System.Text.Encoding.UTF8.GetBytes(text));

    [Fact]
    public void Fnv1aMatchesThePublishedVectors()
    {
        Assert.Equal("cbf29ce484222325", Fnv1a(""));
        Assert.Equal("af63dc4c8601ec8c", Fnv1a("a"));
        Assert.Equal("85944171f73967e8", Fnv1a("foobar"));
    }

    /// <summary>Every crate of the release graph has its row in About, with its licence text.</summary>
    [Fact]
    public void AboutListsEveryRustCrateTheReleaseLinksWithItsLicenceText()
    {
        var crates = RustNotices.Crates;
        Assert.True(crates.Count > 100, "the release links over a hundred crates");
        Assert.Equal($"Rust libraries ({crates.Count})", RustNotices.Heading);
        Assert.Equal(crates.Count, crates.Select(c => c.Id).Distinct().Count()); // each crate once
        Assert.Equal(crates.Select(c => c.Name).Order(StringComparer.Ordinal), crates.Select(c => c.Name)); // in name order
        string[] texts =
        [
            "Permission is hereby granted, free of charge", // MIT
            "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION", // Apache-2.0
            "Redistribution and use in source and binary forms", // BSD
            "provided 'as-is', without any express or implied warranty", // Zlib
        ];
        foreach (var notice in crates)
        {
            Assert.False(notice.Name.StartsWith("ink-", StringComparison.Ordinal), $"{notice.Id}: the workspace's own crates are Inkwell's");
            Assert.True(notice.Text.StartsWith("--- ", StringComparison.Ordinal), $"{notice.Id}: its text starts with a file's name");
            // A licence text, not only a statement naming one.
            Assert.True(texts.Any(t => notice.Text.Contains(t, StringComparison.Ordinal)), $"{notice.Id} ships no licence text");
            if (notice.Shown != notice.Licence)
            {
                Assert.Equal($"{notice.Licence}; used under {notice.Shown}", notice.Detail);
            }
        }
    }

    /// <summary>The Windows target's crates: its own (windows-sys), none of the Mac's. (Windows only.)</summary>
    [Fact]
    public void TheListIsTheWindowsTargetsCrates()
    {
        var names = RustNotices.Crates.Select(c => c.Name).ToHashSet();
        Assert.Contains("windows-sys", names);
        foreach (var name in names)
        {
            Assert.False(
                name.StartsWith("objc2", StringComparison.Ordinal) || name.StartsWith("security-framework", StringComparison.Ordinal)
                || name.StartsWith("core-foundation", StringComparison.Ordinal) || name is "block2" or "apple-native-keyring-store",
                $"{name} is the Mac's");
        }
    }

    /// <summary>
    /// The list was generated from this checkout's Cargo.lock and the release's features: a
    /// dependency change without regenerating fails here. (Its crates are cargo's resolution of the
    /// release build; here, each one is at least in the lock at the version listed.) Windows: its
    /// features are those windows/scripts/build-core.ps1 builds the x64 release with (ink-notices'
    /// WINDOWS_RELEASE_FEATURES; the ARM64 release links a subset of its crates), for the Windows
    /// target.
    /// </summary>
    [Fact]
    public void TheListWasGeneratedFromTheCurrentLockAndTheReleaseFeatures()
    {
        var lockText = AboutCheckout.Read("core/Cargo.lock");
        Assert.True(
            Fnv1a($"{RustNotices.Features}\n{RustNotices.Target}\n{lockText}") == RustNotices.LockFingerprint,
            "RustNotices.g.cs was made from another core/Cargo.lock: run `cargo run -p ink-ffi --bin ink-notices -- --windows`");

        var locked = new HashSet<string>();
        string? name = null;
        foreach (var line in lockText.Split('\n'))
        {
            if (line.StartsWith("name = \"", StringComparison.Ordinal))
            {
                name = line[8..^1];
            }
            else if (line.StartsWith("version = \"", StringComparison.Ordinal) && name is not null)
            {
                locked.Add($"{name} {line[11..^1]}");
                name = null;
            }
        }
        Assert.True(locked.Count > 100, "the lock was read");
        foreach (var notice in RustNotices.Crates)
        {
            Assert.True(locked.Contains(notice.Id), $"{notice.Id} is not in Cargo.lock");
        }

        var script = AboutCheckout.Read("windows/scripts/build-core.ps1");
        Assert.Contains($"X64 = '{RustNotices.Features}'", script, StringComparison.Ordinal);
        Assert.Equal("x86_64-pc-windows-msvc", RustNotices.Target);
    }
}

/// <summary>
/// The notices of the Rust crates in Velopack's Setup.exe and Update.exe (VelopackNotices.g.cs, from
/// `cargo run -p ink-ffi --bin ink-notices -- --velopack`), which About shows under Velopack's
/// notice: made for the Velopack the Windows build pins, from that release's lock.
/// </summary>
public class VelopackNoticesTests
{
    /// <summary>The first quoted value after <paramref name="attr"/>, after <paramref name="key"/>.</summary>
    private static string ValueAfter(string text, string key, string attr)
    {
        var rest = text[text.IndexOf(key, StringComparison.Ordinal)..];
        rest = rest[(rest.IndexOf(attr, StringComparison.Ordinal) + attr.Length)..];
        rest = rest[(rest.IndexOf('"', StringComparison.Ordinal) + 1)..];
        return rest[..rest.IndexOf('"', StringComparison.Ordinal)];
    }

    /// <summary>
    /// The crates are for the Velopack whose Setup.exe and Update.exe the installer ships: the vpk
    /// dotnet-tools.json pins, which is also the Velopack package the app links. A new vpk without
    /// its crates' notices fails here.
    /// </summary>
    [Fact]
    public void TheListIsForTheVelopackTheWindowsBuildPins()
    {
        var tools = AboutCheckout.ReadWindows(".config/dotnet-tools.json");
        Assert.True(
            ValueAfter(tools, "\"vpk\"", "\"version\":") == VelopackNotices.Version,
            "dotnet-tools.json pins another vpk: put its release's files in core/crates/ink-ffi/notices/velopack/ and run `cargo run -p ink-ffi --bin ink-notices -- --velopack`");
        var props = AboutCheckout.ReadWindows("Directory.Packages.props");
        Assert.Equal(VelopackNotices.Version, ValueAfter(props, "Include=\"Velopack\"", "Version="));
        Assert.Equal("x86_64-win7-windows-msvc,aarch64-pc-windows-msvc", VelopackNotices.Target);
        Assert.Equal("windows", VelopackNotices.Features);
    }

    /// <summary>The list was generated from that release's Cargo.lock: each crate is in it at the version listed.</summary>
    [Fact]
    public void TheListWasGeneratedFromVelopacksLock()
    {
        var lockText = AboutCheckout.Read("core/crates/ink-ffi/notices/velopack/Cargo.lock.upstream");
        Assert.True(
            RustNoticesTests.Fnv1a($"{VelopackNotices.Features}\n{VelopackNotices.Target}\n{lockText}") == VelopackNotices.LockFingerprint,
            "VelopackNotices.g.cs was made from another Cargo.lock: run `cargo run -p ink-ffi --bin ink-notices -- --velopack`");
        var locked = new HashSet<string>();
        string? name = null;
        foreach (var line in lockText.Split('\n'))
        {
            if (line.StartsWith("name = \"", StringComparison.Ordinal))
            {
                name = line[8..^1];
            }
            else if (line.StartsWith("version = \"", StringComparison.Ordinal) && name is not null)
            {
                locked.Add($"{name} {line[11..^1]}");
                name = null;
            }
        }
        Assert.True(locked.Count > 500, "the lock was read");
        Assert.All(VelopackNotices.Crates, c => Assert.True(locked.Contains(c.Id), $"{c.Id} is not in Velopack's Cargo.lock"));
    }

    /// <summary>Velopack's row carries every crate's licence files, each under its name, version and licence.</summary>
    [Fact]
    public void AboutShowsEveryCrateUnderVelopacksNotice()
    {
        var crates = VelopackNotices.Crates;
        Assert.True(crates.Count > 100, "Velopack's binaries link over a hundred crates");
        Assert.Equal(crates.Count, crates.Select(c => c.Id).Distinct().Count());
        Assert.Equal(crates.Select(c => c.Name).Order(StringComparer.Ordinal), crates.Select(c => c.Name));
        Assert.Contains(crates, c => c.Name == "webpki-roots" && c.Shown == "CDLA-Permissive-2.0");
        Assert.Contains(crates, c => c.Name == "zerovec" && c.Shown == "Unicode-3.0");
        var velopack = Notices.Components.Single(c => c.Id == "velopack");
        Assert.StartsWith(Notices.VelopackLicence + "\n\n", velopack.Text, StringComparison.Ordinal);
        foreach (var c in crates)
        {
            Assert.False(c.Name.StartsWith("velopack", StringComparison.Ordinal), $"{c.Id}: Velopack's own crates are under its licence");
            Assert.StartsWith("--- ", c.Text, StringComparison.Ordinal);
            Assert.Contains($"=== {c.Title} ({c.Detail}) ===\n{c.Text}", velopack.Text, StringComparison.Ordinal);
        }
        // Microsoft's WebView2 loader, which webview2-com-sys carries, is linked into both.
        Assert.Contains("WebView2LoaderStatic.lib", velopack.Text, StringComparison.Ordinal);
    }
}
