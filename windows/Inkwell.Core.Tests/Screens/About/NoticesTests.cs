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
    /// The notices whose text was not on hand show a placeholder naming the file it comes from;
    /// each must be replaced by that file, verbatim, before a Windows release. This is the list.
    /// </summary>
    [Fact]
    public void EveryPendingNoticeNamesTheFileItsTextComesFrom()
    {
        var pending = Notices.Components.Where(c => c.Pending is not null).ToList();
        Assert.Equal(
            ["wasapi-rs", "windows-app-sdk", "webview2", "winuiex", "dotnet-runtime", "windows-sdk-net"],
            pending.Select(c => c.Id));
        Assert.All(pending, c =>
        {
            Assert.StartsWith("[Pending: ", c.Text, StringComparison.Ordinal);
            Assert.Contains(c.Pending!, c.Text, StringComparison.Ordinal);
        });
        Assert.All(Notices.Components.Where(c => c.Pending is null),
            c => Assert.DoesNotContain("[Pending", c.Text, StringComparison.Ordinal));
    }

    /// <summary>
    /// The texts shared with the Mac are the Mac's, word for word (Notices.swift): every Windows
    /// notice that is not pending is one of the Mac's. (Windows only: the Mac has one list.)
    /// </summary>
    [Fact]
    public void TheSharedTextsAreTheMacs()
    {
        var mac = MacNotices.Texts(AboutCheckout.Read("mac/Sources/Inkwell/Screens/Notices.swift"));
        Assert.True(mac.Count > 15, "Notices.swift was read");
        var shared = Notices.Components.Where(c => c.Pending is null).ToList();
        Assert.Equal(16, shared.Count);
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
    /// Every composed Windows notice has its line and marker. (Windows: the list is the Mac's, so
    /// only its lines for notices the Windows app shows are held to the Windows flags.)
    /// </summary>
    [Fact]
    public void EveryComposedNoticeIsListedWithItsUpstreamCheckAndNothingElseIs()
    {
        var list = AboutCheckout.Read("mac/composed-notices.txt");
        var declared = Notices.ComposedIds;
        Assert.NotEmpty(declared);
        var shown = Notices.Components.Select(c => c.Id).Concat(Notices.Models.Select(m => m.Id)).ToHashSet();
        var ours = string.Join("\n", Lines(list).Where(l => shown.Contains(l.Split(' ', StringSplitOptions.RemoveEmptyEntries)[0])));
        Assert.Equal([], Problems(declared, ours));
        // Every line is well formed (a short line would drop out of the comparison above).
        Assert.All(Lines(list), l => Assert.True(l.Split(' ', StringSplitOptions.RemoveEmptyEntries).Length >= 3, l));
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
