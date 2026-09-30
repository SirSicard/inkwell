// Inkwell 0.2's data on this PC: looked for when the first run shows and each time Settings opens,
// and imported from the first run's step or Settings > Voice. The core knows where 0.2 kept its
// data; the shell asks (import.check, import.run) and says what came back in plain words. After an
// import, ScreenModels reads the key note, the dictation key and Settings' lists again, and the
// Library lists again (import.finished). A port of the Mac's Import02Model.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class Import02Model : ObservableModel
{
    private readonly Action<CoreCommand> send;
    private readonly ScreenLog log;
    private bool asked;

    public Import02Model(Action<CoreCommand> send, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.log = log ?? ScreenLog.System;
    }

    /// <summary>
    /// Whether this app looks for 0.2's data at all: not while the library is moved (INK_DATA_DIR:
    /// development, tests and scripts that must not touch the user's data). The app sets it.
    /// </summary>
    public bool Looks { get; set; } = true;

    /// <summary>What the last check found; null until one answers.</summary>
    public ImportChecked? Found { get; private set; }

    /// <summary>An import is running.</summary>
    public bool Running { get; private set; }

    /// <summary>What this session's import brought.</summary>
    public ImportCounts? Imported { get; private set; }

    /// <summary>Why the last import failed, in words.</summary>
    public string? Failure { get; private set; }

    /// <summary>The last check failed: whether there is anything to import is not known.</summary>
    public bool CheckFailed { get; private set; }

    /// <summary>The ids its commands carry (CoreCommand.ImportCheck, ImportRun).</summary>
    public const string CheckId = "import.check";

    public const string RunId = "import.run";

    /// <summary>The first run's button that moves on without importing.</summary>
    public const string NotNow = "Not now";

    public const string ImportButton = "Import";

    public const string ImportHint = "Brings Inkwell 0.2's history and settings into this library";

    public const string StepTitle = "Your Inkwell 0.2 history";

    /// <summary>Looks for 0.2's data (Settings, each time it opens). Not while an import runs.</summary>
    public void Check()
    {
        if (!Looks || Running)
        {
            return;
        }
        asked = true;
        send(new CoreCommand.ImportCheck());
    }

    /// <summary>Looks once (the first run).</summary>
    public void CheckOnce()
    {
        if (!asked)
        {
            Check();
        }
    }

    /// <summary>Imports (Import). Once per session: after it, there is nothing left to bring over.</summary>
    public void Run()
    {
        if (!Looks || Running || Imported is not null)
        {
            return;
        }
        Running = true;
        Failure = null;
        Changed();
        send(new CoreCommand.ImportRun());
    }

    /// <summary>
    /// Whether the first run shows its step and Settings its row: there is data to import (or data
    /// that cannot be read now), or this session's import to report on.
    /// </summary>
    public bool Offered =>
        Running || Imported is not null || Failure is not null
        || Found?.State is ImportState.Found or ImportState.Unreadable;

    /// <summary>Whether Settings shows its row: offered, or the look for it failed.</summary>
    public bool ShownInSettings => Offered || (CheckFailed && Found is null);

    /// <summary>Whether Import can be pressed.</summary>
    public bool CanImport => Offered && !Running && Imported is null;

    /// <summary>Whether the line is a failure (shown as one).</summary>
    public bool LineIsProblem => CheckFailed && Found is null && Imported is null && !Running;

    /// <summary>Where it stands, in words: what 0.2 left, why it cannot be read, or what came over.</summary>
    public string Line
    {
        get
        {
            if (Imported is { } imported)
            {
                return $"Brought over from Inkwell 0.2: {Summary(imported)}.";
            }
            if (Running)
            {
                return "Bringing over what Inkwell 0.2 left…";
            }
            if (CheckFailed && Found is null)
            {
                return "Couldn’t look for Inkwell 0.2’s data.";
            }
            if (Found is not { } found)
            {
                return "";
            }
            return found.State switch
            {
                ImportState.Found =>
                    $"Inkwell 0.2 left {(found.Counts is { } counts ? Summary(counts) : "its history")} on this PC. Import brings them into this library; 0.2’s own copy stays as it is.",
                ImportState.Unreadable =>
                    $"Inkwell 0.2’s data is on this PC but can’t be read now: {found.Message ?? "no reason was given"}.",
                _ => "",
            };
        }
    }

    /// <summary>Whether this model shows <paramref name="failed"/>: an import's, where it is offered; a check's is logged here.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "import.check" or "import.run";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case ImportChecked checkedNow when checkedNow.Ref == CheckId:
                Found = checkedNow;
                CheckFailed = false;
                Changed();
                break;
            case ImportFinished finished when finished.Ref == RunId:
                Running = false;
                Imported = finished.Counts;
                Failure = null;
                Changed();
                break;
            case CommandFailed { Command: "import.run" } failed when failed.Id == RunId:
                Running = false;
                Failure = $"Couldn’t import: {failed.Message}.";
                Changed();
                break;
            case CommandFailed { Command: "import.check" } failed when failed.Id == CheckId:
                // The first run then goes without the step; Settings says it couldn't look.
                log.Write("import.check failed; the import is not offered");
                CheckFailed = true;
                Changed();
                break;
            default:
                break;
        }
    }

    /// <summary>
    /// The counts in plain words: "1,204 dictations, 3 snippets, 2 modes and your settings".
    /// linked_keys is 0 in a check (the credential store is not asked then), so keys show only after an import.
    /// </summary>
    public static string Summary(ImportCounts counts)
    {
        ArgumentNullException.ThrowIfNull(counts);
        var parts = new List<string>();
        void Add(long n, string one, string many)
        {
            if (n > 0)
            {
                parts.Add($"{n.ToString("N0", CultureInfo.CurrentCulture)} {(n == 1 ? one : many)}");
            }
        }
        Add(counts.Dictations, "dictation", "dictations");
        Add(counts.Snippets, "snippet", "snippets");
        Add(counts.Modes, "mode", "modes");
        Add(counts.VoiceCommands, "voice command", "voice commands");
        Add(counts.DictionaryEntries, "dictionary entry", "dictionary entries");
        Add(counts.AppStyleRules, "app style", "app styles");
        if (counts.Settings > 0)
        {
            parts.Add("your settings");
        }
        // Linked where they are in the credential store, never copied.
        Add(counts.LinkedKeys, "saved API key", "saved API keys");
        return parts.Count switch
        {
            0 => "an empty history",
            1 => parts[0],
            _ => string.Join(", ", parts.Take(parts.Count - 1)) + " and " + parts[^1],
        };
    }
}
