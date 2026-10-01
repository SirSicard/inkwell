// Settings > Modes: each mode with how it writes and the apps it is picked for, each app by its
// name and icon. As the Mac's ModesModel.
//
// A mode names apps by identity, as the core matches them (a lowercase "contains"): on Windows the
// foreground app's identity is its executable's file name (slack.exe), and a mode holds that name
// or part of it. That identity is never shown. An installed app is named by the app directory (its
// display name and icon); one that is not installed is named from a short list of well-known
// apps; failing that, an executable name is shown as its readable stem (examplewriter.exe ->
// Examplewriter), which is also what Windows shows for an app that says nothing better. An id
// from another platform (com.example.app) reads as "An app not on this PC", and a fragment that is
// neither ("slack") is shown as a word.
using System.Collections.Frozen;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>An app as the screen shows it.</summary>
/// <param name="Name">The name the user knows it by. Never an executable name or a bundle id.</param>
/// <param name="IconPath">A file the view reads its icon from (an .exe or .ico), when it is installed.</param>
/// <param name="Installed">Whether it is on this PC.</param>
/// <param name="Id">Unique within one mode's list.</param>
public sealed record AppLabel(string Name, string? IconPath, bool Installed, string Id);

/// <summary>An installed app, as the app directory names it.</summary>
/// <param name="IconPath">Where the view reads its icon (its .exe, or an .ico), if known.</param>
public sealed record InstalledApp(string Name, string? IconPath);

/// <summary>Finds installed apps.</summary>
public interface IAppDirectory
{
    /// <summary>
    /// The installed app whose executable is <paramref name="exe"/> (a file name such as
    /// slack.exe, in any case): its display name and icon; null when none is installed.
    /// </summary>
    InstalledApp? App(string exe);
}

/// <summary>
/// No installed apps known: every app is named from the well-known list or its stem. The pure
/// fallback, and what tests use; the app passes a directory that reads what is installed.
/// </summary>
public sealed class NoInstalledApps : IAppDirectory
{
    public static NoInstalledApps Instance { get; } = new();

    public InstalledApp? App(string exe) => null;
}

/// <summary>Names an app identity for the screen.</summary>
public static class AppIdentity
{
    /// <summary>Well-known apps by executable name (any case), for an identity whose app is not installed here.</summary>
    public static FrozenDictionary<string, string> Known { get; } = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
    {
        ["slack.exe"] = "Slack",
        ["whatsapp.exe"] = "WhatsApp",
        ["whatsapp.root.exe"] = "WhatsApp",
        ["code.exe"] = "VS Code",
        ["ms-teams.exe"] = "Microsoft Teams",
        ["teams.exe"] = "Microsoft Teams",
        ["outlook.exe"] = "Outlook",
        ["olk.exe"] = "Outlook",
        ["winword.exe"] = "Word",
        ["excel.exe"] = "Excel",
        ["powerpnt.exe"] = "PowerPoint",
        ["onenote.exe"] = "OneNote",
        ["zoom.exe"] = "Zoom",
        ["chrome.exe"] = "Chrome",
        ["msedge.exe"] = "Microsoft Edge",
        ["firefox.exe"] = "Firefox",
        ["brave.exe"] = "Brave",
        ["discord.exe"] = "Discord",
        ["telegram.exe"] = "Telegram",
        ["notion.exe"] = "Notion",
        ["linear.exe"] = "Linear",
        ["figma.exe"] = "Figma",
        ["notepad.exe"] = "Notepad",
        ["windowsterminal.exe"] = "Windows Terminal",
        ["powershell.exe"] = "PowerShell",
        ["pwsh.exe"] = "PowerShell",
        ["cmd.exe"] = "Command Prompt",
        ["devenv.exe"] = "Visual Studio",
        ["explorer.exe"] = "File Explorer",
    }.ToFrozenDictionary(StringComparer.OrdinalIgnoreCase);

    /// <summary>Whether <paramref name="identity"/> is an executable's file name.</summary>
    public static bool IsExe(string identity)
    {
        ArgumentNullException.ThrowIfNull(identity);
        return identity.Length > 4 && identity.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) && !identity.Contains(' ', StringComparison.Ordinal);
    }

    /// <summary>Whether <paramref name="identity"/> looks like another platform's app id (a bundle id) rather than a word or an executable.</summary>
    public static bool IsForeignId(string identity)
    {
        ArgumentNullException.ThrowIfNull(identity);
        return !IsExe(identity) && identity.Contains('.', StringComparison.Ordinal) && !identity.Contains(' ', StringComparison.Ordinal);
    }

    /// <summary><paramref name="identity"/> as the user should see it.</summary>
    public static AppLabel Label(string identity, IAppDirectory apps)
    {
        ArgumentNullException.ThrowIfNull(identity);
        ArgumentNullException.ThrowIfNull(apps);
        var trimmed = identity.Trim();
        if (IsExe(trimmed) && apps.App(trimmed) is InstalledApp app)
        {
            return new AppLabel(app.Name, app.IconPath, Installed: true, trimmed);
        }
        if (Known.TryGetValue(trimmed, out var known))
        {
            return new AppLabel(known, null, Installed: false, trimmed);
        }
        if (IsExe(trimmed))
        {
            // Not installed and not well known: its stem, as Windows names an app that says nothing better.
            return new AppLabel(Capitalised(trimmed[..^4]), null, Installed: false, trimmed);
        }
        if (IsForeignId(trimmed))
        {
            return new AppLabel("An app not on this PC", null, Installed: false, trimmed);
        }
        // A fragment the core matches inside identities: shown as the word it is.
        return new AppLabel(Capitalised(trimmed), null, Installed: false, trimmed);
    }

    private static string Capitalised(string word) =>
        word.Length == 0 ? word : string.Concat(word[..1].ToUpperInvariant(), word[1..]);
}

/// <summary>One mode's row.</summary>
/// <param name="Traits">How it writes: its style, then "Clean up speech" and "Polish" where on.</param>
/// <param name="Apps">The apps it is picked for, by name.</param>
public sealed record ModeRow(string Id, string Name, bool IsDefault, IReadOnlyList<string> Traits, IReadOnlyList<AppLabel> Apps)
{
    /// <summary>The apps' names in one line, or what an empty list means.</summary>
    public string AppsText => Apps.Count == 0
        ? (IsDefault ? "Every app without a mode of its own" : "No apps")
        : string.Join(", ", Apps.Select(a => a.Name));

    /// <summary>The row read aloud.</summary>
    public string AccessibilityLabel
    {
        get
        {
            var apps = Apps.Count == 0 ? (IsDefault ? "the default" : "no apps") : string.Join(", ", Apps.Select(a => a.Name));
            return $"{Name}: {string.Join(", ", Traits)}; used in {apps}";
        }
    }

    public bool Equals(ModeRow? other) =>
        other is not null && Id == other.Id && Name == other.Name && IsDefault == other.IsDefault
        && Traits.SequenceEqual(other.Traits) && Apps.SequenceEqual(other.Apps);

    public override int GetHashCode() => HashCode.Combine(Id, Name, IsDefault, Traits.Count, Apps.Count);
}

public sealed class ModesModel(Action<CoreCommand> send, IAppDirectory? apps = null) : ObservableModel
{
    public const string FailedText = "Your modes could not be read.";

    private readonly IAppDirectory apps = apps ?? NoInstalledApps.Instance;

    public IReadOnlyList<ModeRow> Rows { get; private set; } = [];

    /// <summary>The modes could not be read.</summary>
    public bool Failed { get; private set; }

    public void Load() => send(new CoreCommand.ModesList());

    public static string Style(ModeStyle style) => style switch
    {
        ModeStyle.Formal => "Formal",
        ModeStyle.Casual => "Casual",
        ModeStyle.Relaxed => "Relaxed",
        _ => "Own style",
    };

    /// <summary>A row's title: the default mode, listed last among others, is "Everywhere else".</summary>
    public string Title(ModeRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        return row.IsDefault && Rows.Count > 1 ? "Everywhere else" : row.Name;
    }

    /// <summary>Whether this screen shows the failure (modes.list).</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command == "modes.list";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case ModesListed listed:
                Failed = false;
                // The default mode last, as "everywhere else": the others are matched first.
                var others = listed.Modes.Where(m => m.Id != listed.DefaultId);
                var fallback = listed.Modes.Where(m => m.Id == listed.DefaultId);
                Rows = others.Concat(fallback).Select(mode => Row(mode, listed.DefaultId)).ToList();
                Changed();
                break;
            case CommandFailed failed when failed.Command == "modes.list":
                Failed = true;
                Changed();
                break;
            default:
                break;
        }
    }

    private ModeRow Row(ModeInfo mode, string defaultId)
    {
        var traits = new List<string> { Style(mode.Style) };
        if (mode.RemoveFillers)
        {
            traits.Add("Clean up speech");
        }
        if (mode.Polish)
        {
            traits.Add("Polish");
        }
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var labels = mode.Apps
            .Where(a => !string.IsNullOrWhiteSpace(a))
            .Select(a => AppIdentity.Label(a, apps))
            .Where(label => seen.Add(label.Id))
            .ToList();
        return new ModeRow(mode.Id, mode.Name, mode.Id == defaultId, traits, labels);
    }
}
