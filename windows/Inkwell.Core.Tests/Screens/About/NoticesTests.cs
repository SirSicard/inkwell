// About's notices on Windows (the Mac's NoticesTests, ComposedNoticesTests and RustNoticesTests):
// every component the Windows app ships has its notice, nothing Mac-only appears, the texts shared
// with the Mac are the Mac's, and the generated Rust list is the one made from this checkout's
// Cargo.lock for the Windows target.
//
// Tests that read the repository (THIRD_PARTY.md, core/Cargo.lock, the Mac's files) skip, saying
// so, where the tests run from a copy of windows/ alone; in the checkout they always run.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

/// <summary>The repository the tests were built from, for the tests that read it.</summary>
internal static class AboutCheckout
{
    /// <summary>The repository's root: the nearest directory above the tests with THIRD_PARTY.md and core/Cargo.lock.</summary>
    public static string Root()
    {
        for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir is not null; dir = dir.Parent)
        {
            if (File.Exists(Path.Combine(dir.FullName, "THIRD_PARTY.md"))
                && File.Exists(Path.Combine(dir.FullName, "core", "Cargo.lock")))
            {
                return dir.FullName;
            }
        }
        Assert.Skip("needs the repository checkout (THIRD_PARTY.md, core/Cargo.lock); these tests run from a copy of windows/ alone");
        throw new InvalidOperationException("unreachable");
    }

    /// <summary>A repository file's text, line ends normalised.</summary>
    public static string Read(string relative) =>
        File.ReadAllText(Path.Combine(Root(), relative)).Replace("\r\n", "\n", StringComparison.Ordinal);

    /// <summary>
    /// A file of the Windows solution's folder (windows/, found by Inkwell.slnx above the tests),
    /// line ends normalised. Every copy of the solution has it, so these never skip.
    /// </summary>
    public static string ReadWindows(string relative)
    {
        for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir is not null; dir = dir.Parent)
        {
            if (File.Exists(Path.Combine(dir.FullName, "Inkwell.slnx")))
            {
                return File.ReadAllText(Path.Combine(dir.FullName, relative)).Replace("\r\n", "\n", StringComparison.Ordinal);
            }
        }
        throw new DirectoryNotFoundException("no Inkwell.slnx above the tests");
    }
}

public class NoticesTests
{
    /// <summary>THIRD_PARTY.md's rows that reach only the Mac app (or neither app), by name.</summary>
    private static readonly string[] NotShippedOnWindows =
    [
        "Handy", // the legacy 0.2 app's base
        "AudioCap", "FluidAudio", "fastcluster", "VBx", "Sparkle", // the Mac's
    ];

    private static string About =>
        string.Join("\n", Notices.Components.Select(c => c.Name + " " + c.Role + " " + c.Text)
            .Concat(Notices.Models.Select(m => m.Name + " " + m.Use + " " + m.Notice)));

    /// <summary>
    /// Every THIRD_PARTY.md row that reaches the Windows app has its notice in About. (Windows: the
    /// Mac-only rows are left out by name, and must not appear.)
    /// </summary>
    [Fact]
    public void AboutCarriesTheNoticeOfEveryComponentTheAppShips()
    {
        var rows = AboutCheckout.Read("THIRD_PARTY.md").Split('\n')
            .Where(r => r.StartsWith("| ", StringComparison.Ordinal) && !r.StartsWith("| Project", StringComparison.Ordinal));
        var names = rows.Select(row =>
        {
            var cell = row.Split('|')[1].Trim();
            // "[Name](url) by Author" or "Name, by Author": the name only.
            return cell.StartsWith('[') ? cell[1..cell.IndexOf(']', StringComparison.Ordinal)] : cell.Split(',')[0];
        }).Where(n => n.Length > 0).ToList();
        Assert.True(names.Count > 15, "the table was read");
        foreach (var gone in NotShippedOnWindows)
        {
            Assert.Contains(gone, names); // a renamed row would otherwise pass unseen
        }
        foreach (var name in names.Except(NotShippedOnWindows))
        {
            var key = name.Replace(" clustering", "", StringComparison.Ordinal).Replace(" in C", "", StringComparison.Ordinal);
            Assert.True(About.Contains(key, StringComparison.OrdinalIgnoreCase), $"About has no notice for {name}");
        }
        var ids = Notices.Components.Select(c => c.Id).ToList();
        Assert.Equal(ids.Count, ids.Distinct().Count());
        Assert.Contains("NVIDIA CORPORATION & AFFILIATES", Notices.Components.Single(c => c.Id == "nemo-speech").Text, StringComparison.Ordinal);
        Assert.Contains("patent", Notices.Components.Single(c => c.Id == "aec3").Text, StringComparison.OrdinalIgnoreCase);
        Assert.All(Notices.Components, c => Assert.False(string.IsNullOrWhiteSpace(c.Text), c.Id));
        var parakeet = Notices.Models.Single(m => m.Id == "parakeet");
        Assert.Equal("CC-BY-4.0", parakeet.Licence);
        Assert.NotNull(parakeet.Notice); // CC-BY needs its credit
    }

    /// <summary>What only the Windows app ships has its notice: the capture's wasapi-rs pattern, the UI stack and the runtime.</summary>
    [Fact]
    public void AboutCarriesTheWindowsComponents()
    {
        var byId = Notices.Components.ToDictionary(c => c.Id);
        Assert.Equal("MIT", byId["wasapi-rs"].Licence);
        Assert.Contains("wasapi-rs", byId["wasapi-rs"].Name, StringComparison.Ordinal);
        Assert.Contains("Microsoft.WindowsAppSDK.WinUI 2.3.9", byId["windows-app-sdk"].Role, StringComparison.Ordinal);
        foreach (var part in new[] { "Base", "Foundation", "InteractiveExperiences" })
        {
            Assert.Contains(part, byId["windows-app-sdk"].Role, StringComparison.Ordinal);
        }
        Assert.Equal("BSD-3-Clause", byId["webview2"].Licence);
        Assert.Contains("1.0.3719.77", byId["webview2"].Role, StringComparison.Ordinal);
        Assert.Contains("2.9.3", byId["winuiex"].Role, StringComparison.Ordinal);
        Assert.StartsWith("MIT", byId["dotnet-runtime"].Licence, StringComparison.Ordinal);
        Assert.Contains("windows-sdk-net", byId.Keys);
        Assert.Equal("MIT", byId["cswinrt"].Licence);

        // The texts are the packages' own (NoticeTexts.cs).
        Assert.StartsWith("Copyright (c) 2020 Henrik Enquist", byId["wasapi-rs"].Text, StringComparison.Ordinal);
        Assert.Contains("MICROSOFT SOFTWARE LICENSE TERMS\nMICROSOFT WINDOWS APP SDK", byId["windows-app-sdk"].Text, StringComparison.Ordinal);
        Assert.Contains("--- NOTICE.txt (Microsoft.WindowsAppSDK.Base 2.0.4) ---", byId["windows-app-sdk"].Text, StringComparison.Ordinal);
        Assert.Contains("Copyright (C) Microsoft Corporation. All rights reserved.", byId["webview2"].Text, StringComparison.Ordinal);
        Assert.Contains("--- NOTICE.txt ---", byId["webview2"].Text, StringComparison.Ordinal);
        Assert.Contains("Copyright \u00a9 2021-2026 - Morten Nielsen", byId["winuiex"].Text, StringComparison.Ordinal);
        Assert.Contains("Copyright (c) .NET Foundation and Contributors", byId["dotnet-runtime"].Text, StringComparison.Ordinal);
        Assert.Contains("--- THIRD-PARTY-NOTICES.TXT ---", byId["dotnet-runtime"].Text, StringComparison.Ordinal);
        Assert.Contains("https://aka.ms/WinSDKLicenseURL", byId["windows-sdk-net"].Text, StringComparison.Ordinal);
        Assert.Contains("Copyright (c) Microsoft Corporation.", byId["cswinrt"].Text, StringComparison.Ordinal);

        // Windows' Parakeet runs on sherpa-onnx and the ONNX Runtime its archive carries.
        Assert.Equal("Apache-2.0", byId["sherpa-onnx"].Licence);
        Assert.Equal(Notices.Apache2, byId["sherpa-onnx"].Text);
        Assert.Contains("1.13.4", byId["sherpa-onnx"].Role, StringComparison.Ordinal);
        Assert.Equal("MIT", byId["onnxruntime"].Licence);
        Assert.StartsWith("MIT License\n\nCopyright (c) Microsoft Corporation\n", byId["onnxruntime"].Text, StringComparison.Ordinal);
        Assert.Contains("1.27.0", byId["onnxruntime"].Role, StringComparison.Ordinal);
    }

    /// <summary>
    /// The Microsoft licences About shows are the files the NuGet licence check approved: their
    /// SHA-256 (LF line ends) is the one windows/scripts/nuget-licence-exceptions.json pins. (Windows only.)
    /// </summary>
    [Fact]
    public void TheMicrosoftLicencesAreTheFilesTheLicenceCheckApproved()
    {
        using var exceptions = System.Text.Json.JsonDocument.Parse(AboutCheckout.ReadWindows("scripts/nuget-licence-exceptions.json"));
        var pinned = exceptions.RootElement.EnumerateArray()
            .ToDictionary(e => e.GetProperty("id").GetString()!, e => e.GetProperty("licence").GetString()!);
        static string Sha(string text) =>
            Convert.ToHexStringLower(System.Security.Cryptography.SHA256.HashData(System.Text.Encoding.UTF8.GetBytes(text)));
        foreach (var id in new[] { "Microsoft.WindowsAppSDK.WinUI", "Microsoft.WindowsAppSDK.Base", "Microsoft.WindowsAppSDK.Foundation", "Microsoft.WindowsAppSDK.InteractiveExperiences" })
        {
            Assert.Equal($"file license.txt sha256:{Sha(Notices.WindowsAppSdkLicence)}", pinned[id]);
        }
        Assert.Equal($"file LICENSE.txt sha256:{Sha(Notices.WebView2Licence)}", pinned["Microsoft.Web.WebView2"]);
    }

    /// <summary>Nothing Mac-only: no component, model credit or text names the Mac's own pieces.</summary>
    [Fact]
    public void NothingMacOnlyAppears()
    {
        foreach (var id in new[] { "audiocap", "fluidaudio", "fastcluster", "vbx", "sparkle" })
        {
            Assert.DoesNotContain(id, Notices.Components.Select(c => c.Id));
        }
        foreach (var word in new[] { "AudioCap", "FluidAudio", "FluidInference", "Sparkle", "Core ML", "Neural Engine", "Contents/Frameworks" })
        {
            Assert.False(About.Contains(word, StringComparison.OrdinalIgnoreCase), $"About names {word}");
        }
    }

    /// <summary>
    /// No notice ships as a placeholder: every text is the component's own (or composed, and
    /// listed as such). This was the list of pending notices; it must stay empty.
    /// </summary>
    [Fact]
    public void NoNoticeIsPending()
    {
        Assert.Empty(Notices.Components.Where(c => c.Pending is not null).Select(c => c.Id));
        Assert.All(Notices.Components, c => Assert.DoesNotContain("[Pending", c.Text, StringComparison.Ordinal));
    }

    /// <summary>
    /// The texts shared with the Mac are the Mac's, word for word (Notices.swift): every Windows
    /// notice but the Windows-only ones is one of the Mac's. (Windows only: the Mac has one list.)
    /// </summary>
    [Fact]
    public void TheSharedTextsAreTheMacs()
    {
        var mac = MacNotices.Texts(AboutCheckout.Read("mac/Sources/Inkwell/Screens/Notices.swift"));
        Assert.True(mac.Count > 15, "Notices.swift was read");
        string[] windowsOnly = ["wasapi-rs", "sherpa-onnx", "onnxruntime", "windows-app-sdk", "webview2", "winuiex", "dotnet-runtime", "windows-sdk-net", "cswinrt"];
        var shared = Notices.Components.Where(c => !windowsOnly.Contains(c.Id)).ToList();
        Assert.Equal(16, shared.Count);
        Assert.All(windowsOnly, id => Assert.False(mac.ContainsKey(id), id));
        foreach (var c in shared)
        {
            Assert.True(mac.ContainsKey(c.Id), $"{c.Id} is not one of the Mac's notices");
            Assert.True(mac[c.Id] == c.Text, $"{c.Id}: the text differs from Notices.swift's");
        }
        Assert.Equal(mac["silero-vad"], Notices.Models.Single(m => m.Id == "silero-vad").Notice);
    }
}

/// <summary>The licence texts of the Mac's Notices.swift, by notice id (a raw literal, or a shared one it names).</summary>
internal static class MacNotices
{
    private const string Raw = "#\"\"\"";

    public static Dictionary<string, string> Texts(string swift)
    {
        var lines = swift.Split('\n');
        var statics = new Dictionary<string, string>();
        var texts = new Dictionary<string, string>();
        var named = new Dictionary<string, string>();
        string? id = null;
        for (var i = 0; i < lines.Length; i++)
        {
            var line = lines[i].Trim();
            var at = line.IndexOf("id: \"", StringComparison.Ordinal);
            if (at >= 0)
            {
                var start = at + 5;
                id = line[start..line.IndexOf('"', start)];
            }
            if (line.StartsWith("static let ", StringComparison.Ordinal) && line.EndsWith(" = " + Raw, StringComparison.Ordinal))
            {
                statics[line["static let ".Length..^(" = " + Raw).Length]] = ReadRaw(lines, ref i);
            }
            else if (line == "text: " + Raw && id is not null)
            {
                texts[id] = ReadRaw(lines, ref i);
            }
            else if (id is not null && Word(line, "text: ") is { } text)
            {
                named[id] = text;
            }
            else if (id is not null && line.Contains("notice: ", StringComparison.Ordinal)
                && Word(line[line.IndexOf("notice: ", StringComparison.Ordinal)..], "notice: ") is { } notice && notice != "nil")
            {
                named[id] = notice;
            }
        }
        foreach (var (key, name) in named)
        {
            texts[key] = statics[name];
        }
        return texts;
    }

    /// <summary>The identifier after <paramref name="prefix"/>, up to "," or ")"; null if there is none.</summary>
    private static string? Word(string line, string prefix)
    {
        if (!line.StartsWith(prefix, StringComparison.Ordinal))
        {
            return null;
        }
        var word = new string(line[prefix.Length..].TakeWhile(char.IsLetterOrDigit).ToArray());
        return word.Length > 0 ? word : null;
    }

    /// <summary>A raw literal's lines, from the one after its opening to the one before its closing ("""#).</summary>
    private static string ReadRaw(string[] lines, ref int i)
    {
        var body = new List<string>();
        for (i++; !lines[i].StartsWith("\"\"\"#", StringComparison.Ordinal); i++)
        {
            body.Add(lines[i]);
        }
        return string.Join("\n", body);
    }
}

/// <summary>
/// The notices composed from a licence's standard text say so (<c>Composed</c>), and
/// mac/composed-notices.txt lists each with its upstream-check marker, which a release waits for
/// (mac/scripts/notices-verified.sh). The Windows notices are copies of the Mac's, so the same list
/// holds them.
/// </summary>
public class ComposedNoticesTests
{
    /// <summary>The list's lines, by notice id: <c>&lt;id&gt; verified=&lt;no|YYYY-MM-DD&gt; &lt;what to compare with&gt;</c>.</summary>
    private static Dictionary<string, string> Markers(string list)
    {
        var listed = new Dictionary<string, string>();
        foreach (var line in Lines(list))
        {
            var words = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
            if (words.Length >= 3)
            {
                listed[words[0]] = words[1];
            }
        }
        return listed;
    }

    private static IEnumerable<string> Lines(string list) =>
        list.Split('\n').Where(l => !l.StartsWith('#') && l.Trim().Length > 0);

    /// <summary>
    /// What is wrong between the composed notices and the list: one declared without a line, one
    /// listed that is not declared, and a marker that is neither <c>verified=no</c> nor a date.
    /// </summary>
    private static List<string> Problems(IReadOnlySet<string> declared, string list)
    {
        var listed = Markers(list);
        var out_ = declared.Except(listed.Keys).Order(StringComparer.Ordinal)
            .Select(id => $"{id} is composed but has no line (no verified= marker)").ToList();
        out_.AddRange(listed.Keys.Except(declared).Order(StringComparer.Ordinal)
            .Select(id => $"{id} is listed but not composed in Notices.swift"));
        out_.AddRange(listed.OrderBy(kv => kv.Key, StringComparer.Ordinal)
            .Where(kv => kv.Value != "verified=no" && !IsDate(kv.Value))
            .Select(kv => $"{kv.Key}: {kv.Value}"));
        return out_;
    }

    private static bool IsDate(string marker) =>
        marker.StartsWith("verified=", StringComparison.Ordinal)
        && marker["verified=".Length..] is { Length: 10 } d
        && d.Select((c, i) => i is 4 or 7 ? c == '-' : char.IsAsciiDigit(c)).All(ok => ok);

    /// <summary>
    /// Every composed Windows notice has its line and marker. (Windows: the shared notices' lines
    /// are the Mac's list, of which only the lines for notices the Windows app shows are held to
    /// the Windows flags; the Windows-only ones are in About/composed-notices.txt.)
    /// </summary>
    [Fact]
    public void EveryComposedNoticeIsListedWithItsUpstreamCheckAndNothingElseIs()
    {
        var mac = AboutCheckout.Read("mac/composed-notices.txt");
        var windows = AboutCheckout.ReadWindows("Inkwell.Core/Screens/About/composed-notices.txt");
        var declared = Notices.ComposedIds;
        Assert.NotEmpty(declared);
        var shown = Notices.Components.Select(c => c.Id).Concat(Notices.Models.Select(m => m.Id)).ToHashSet();
        var ours = string.Join("\n", Lines(mac).Where(l => shown.Contains(l.Split(' ', StringSplitOptions.RemoveEmptyEntries)[0])));
        Assert.Equal([], Problems(declared, ours + "\n" + windows));
        // Every line is well formed (a short line would drop out of the comparison above).
        Assert.All(Lines(mac).Concat(Lines(windows)), l => Assert.True(l.Split(' ', StringSplitOptions.RemoveEmptyEntries).Length >= 3, l));
        Assert.Equal(["winuiex", "windows-sdk-net", "onnxruntime"], Markers(windows).Keys);
    }

    /// <summary>The check fails when a composed notice has no line, and when a line names no composed notice.</summary>
    [Fact]
    public void ACheckThatCatchesAComposedNoticeWithoutItsMarker()
    {
        const string list = "# header\nprotobuf-lite  verified=no  its file\nsilero-vad verified=2026-10-04 its file\n";
        Assert.Equal([], Problems(new HashSet<string> { "protobuf-lite", "silero-vad" }, list));
        Assert.Equal(
            ["new-one is composed but has no line (no verified= marker)"],
            Problems(new HashSet<string> { "protobuf-lite", "silero-vad", "new-one" }, list));
        Assert.Equal(
            ["silero-vad is listed but not composed in Notices.swift"],
            Problems(new HashSet<string> { "protobuf-lite" }, list));
        Assert.Equal(["x: verified=soon"], Problems(new HashSet<string> { "x" }, "x verified=soon its file"));
    }
}
