// Settings > Snippets and Settings > Voice commands: the lists dictation uses, as the core stores
// them (the user's own, else the ones the Inkwell 0.2 import brought), and what became of 0.2's
// dictation key, said once. As the Mac's PhrasesModel (and the logic of its PhrasesSections).
//
// The core is the source of truth: each list arrives whole (snippets.listed,
// voice_commands.listed) and a change sends the whole list back (snippets.save,
// voice_commands.save), which a running dictation takes at once. The row changes at once on screen;
// the core's answer then replaces it. A save that fails says so and reads the list again, so the
// screen never shows what was not saved. Nothing can be changed until a list has been read: a
// stored list the core cannot read is never replaced by an edit, only by "Start over", which the
// user chooses (and the core refuses any other save over it). The user's words travel in these
// lists: never log them.
using System.Collections.Immutable;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>What the list sections share: the texts, and a new id for a row the user adds.</summary>
public static class PhraseLists
{
    public const string SaveFailedText = "Couldn’t save that change. The list shows what is saved.";
    public const string FromImportText = "Brought over from Inkwell 0.2.";
    public const string StartOverTitle = "Start over (replaces the damaged list)";
    public const string StartOverHelp = "The stored list can’t be read. This replaces it with an empty one.";

    internal static string NewId(string prefix) => $"{prefix}-{Guid.NewGuid():D}";
}

public sealed class SnippetsModel(Action<CoreCommand> send) : ObservableModel
{
    public const string LoadFailedText = "Couldn’t read your snippets.";
    public const string SaveFailedText = PhraseLists.SaveFailedText;
    public const string RefPrefix = "snippets:";
    public const string EmptyText = "No snippets yet.";
    public const string HelpText = "A trigger is matched as whole words, in any case, after the dictionary. {date} and {time} in the text are filled in when it goes in.";

    private int nextRef;
    /// <summary>
    /// The newest list or save sent: only its answer replaces the rows, so an answer to an earlier
    /// save never shows over a later change for a moment.
    /// </summary>
    private string? latest;

    public IReadOnlyList<SnippetDraft> Rows { get; private set; } = [];

    /// <summary>The list is the one the 0.2 import brought, not yet saved in 1.0.</summary>
    public bool FromImport { get; private set; }

    /// <summary>The list has been read: until then, and after a read fails, nothing can be changed.</summary>
    public bool Loaded { get; private set; }

    /// <summary>The stored list could not be read: only "Start over" can replace it.</summary>
    public bool Unreadable { get; private set; }

    /// <summary>What went wrong, in words.</summary>
    public string? Failure { get; private set; }

    private string NextRef()
    {
        nextRef++;
        var id = $"{RefPrefix}{nextRef}";
        latest = id;
        return id;
    }

    public void Load() => send(new CoreCommand.SnippetsList(NextRef()));

    /// <summary>Adds a snippet; a blank trigger adds nothing (the caller keeps Add disabled then).</summary>
    public void Add(string trigger, string expansion, string category)
    {
        ArgumentNullException.ThrowIfNull(trigger);
        ArgumentNullException.ThrowIfNull(expansion);
        ArgumentNullException.ThrowIfNull(category);
        trigger = trigger.Trim();
        if (!Loaded || trigger.Length == 0)
        {
            return;
        }
        Save([.. Rows, new SnippetDraft(PhraseLists.NewId("snip"), trigger, expansion, category.Trim())]);
    }

    /// <summary>Replaces the snippet with <paramref name="draft"/>'s id; a blank trigger changes nothing.</summary>
    public void Update(SnippetDraft draft)
    {
        ArgumentNullException.ThrowIfNull(draft);
        draft = draft with { Trigger = draft.Trigger.Trim() };
        var i = Rows.ToList().FindIndex(r => r.Id == draft.Id);
        if (!Loaded || draft.Trigger.Length == 0 || i < 0)
        {
            return;
        }
        var next = Rows.ToList();
        next[i] = draft;
        Save(next);
    }

    public void SetEnabled(string id, bool on)
    {
        if (Rows.FirstOrDefault(r => r.Id == id) is SnippetDraft row)
        {
            Update(row with { Enabled = on });
        }
    }

    public void Delete(string id)
    {
        if (!Loaded)
        {
            return;
        }
        Save(Rows.Where(r => r.Id != id).ToList());
    }

    /// <summary>Replaces a stored list that cannot be read with an empty one: only when the user chooses it.</summary>
    public void StartOver()
    {
        if (!Unreadable)
        {
            return;
        }
        send(new CoreCommand.SnippetsSave([], ReplaceUnreadable: true, NextRef()));
    }

    private void Save(IReadOnlyList<SnippetDraft> next)
    {
        if (!Loaded)
        {
            return;
        }
        Rows = next;
        Failure = null;
        Changed();
        send(new CoreCommand.SnippetsSave(next, ReplaceUnreadable: false, NextRef()));
    }

    /// <summary>Whether this screen shows the failure: every snippets.list and snippets.save.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "snippets.list" or "snippets.save";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case SnippetsListed listed when latest is null || listed.Ref is null || listed.Ref == latest:
                Rows = listed.Snippets.Select(s => new SnippetDraft(s)).ToList();
                FromImport = listed.FromImport;
                Loaded = true;
                Unreadable = false;
                Failure = null;
                Changed();
                break;
            case CommandFailed failed when failed.Command == "snippets.list" && (failed.Id is null || failed.Id == latest):
                // Not known what is stored: nothing is shown, and nothing can be changed.
                ReadFailed();
                break;
            case CommandFailed failed when failed.Command == "snippets.save":
                // The core refused to save over a stored list it cannot read: told apart by the code.
                if (failed.Code == FailureCode.ListUnreadable)
                {
                    // The stored list became unreadable after it was read: say so, and offer Start
                    // over, at once (the read below fails the same way).
                    ReadFailed();
                }
                else
                {
                    Failure = SaveFailedText;
                    Changed();
                }
                send(new CoreCommand.SnippetsList(NextRef()));
                break;
            default:
                break;
        }
    }

    private void ReadFailed()
    {
        Rows = [];
        Loaded = false;
        Unreadable = true;
        Failure = LoadFailedText;
        Changed();
    }
}

public sealed class VoiceCommandsModel(Action<CoreCommand> send) : ObservableModel
{
    public const string LoadFailedText = "Couldn’t read your voice commands.";
    public const string SaveFailedText = PhraseLists.SaveFailedText;
    public const string RefPrefix = "voice_commands:";
    public const string NotCarriedOutText = "Not available in this version: Inkwell hears it and types nothing.";

    /// <summary>
    /// The kinds a new command can be: the ones this build carries out, bar polish (which has its
    /// own switch and consent in Settings > AI).
    /// </summary>
    public static ImmutableArray<CommandAction> Addable { get; } = [CommandAction.InsertText, CommandAction.ChangeStyle];

    private int nextRef;
    /// <summary>As in SnippetsModel.</summary>
    private string? latest;

    public bool Enabled { get; private set; }

    public string WakePrefix { get; private set; } = "inkwell";

    public IReadOnlyList<VoiceCommandDraft> Rows { get; private set; } = [];

    public bool FromImport { get; private set; }

    /// <summary>As in SnippetsModel.</summary>
    public bool Loaded { get; private set; }

    public bool Unreadable { get; private set; }

    public string? Failure { get; private set; }

    /// <summary>The line beside the switch.</summary>
    public string StatusLine => Enabled
        ? $"Say “{WakePrefix}”, then a command"
        : "Off: everything you say is dictated";

    private string NextRef()
    {
        nextRef++;
        var id = $"{RefPrefix}{nextRef}";
        latest = id;
        return id;
    }

    public void Load() => send(new CoreCommand.VoiceCommandsList(NextRef()));

    public void SetEnabled(bool on)
    {
        if (!Loaded)
        {
            return;
        }
        Save(on, WakePrefix, Rows);
    }

    /// <summary>Replaces stored commands that cannot be read with none, switched off: only when the user chooses it.</summary>
    public void StartOver()
    {
        if (!Unreadable)
        {
            return;
        }
        send(new CoreCommand.VoiceCommandsSave(false, "inkwell", [], ReplaceUnreadable: true, NextRef()));
    }

    /// <summary>A blank wake word changes nothing (it would match nothing).</summary>
    public void SetWakePrefix(string word)
    {
        ArgumentNullException.ThrowIfNull(word);
        word = word.Trim().ToLowerInvariant();
        if (!Loaded || word.Length == 0 || word == WakePrefix)
        {
            return;
        }
        Save(Enabled, word, Rows);
    }

    /// <summary>Whether the wake word's Save does anything with <paramref name="word"/> typed.</summary>
    public bool CanSaveWakePrefix(string word)
    {
        ArgumentNullException.ThrowIfNull(word);
        var trimmed = word.Trim().ToLowerInvariant();
        return Loaded && trimmed.Length > 0 && trimmed != WakePrefix;
    }

    public void SetCommandEnabled(string id, bool on)
    {
        if (!Loaded)
        {
            return;
        }
        Save(Enabled, WakePrefix, Rows.Select(r => r.Id == id ? r with { Enabled = on } : r).ToList());
    }

    public void Delete(string id)
    {
        if (!Loaded)
        {
            return;
        }
        Save(Enabled, WakePrefix, Rows.Where(r => r.Id != id).ToList());
    }

    /// <summary>
    /// Adds a command: <paramref name="triggers"/> comma-separated, <paramref name="value"/> the
    /// text to type or the style's name. Nothing is added without a trigger and a value.
    /// </summary>
    public void Add(string triggers, CommandAction action, string value)
    {
        ArgumentNullException.ThrowIfNull(value);
        var phrases = Phrases(triggers);
        value = value.Trim();
        if (!Loaded || phrases.Count == 0 || value.Length == 0 || !Addable.Contains(action))
        {
            return;
        }
        var row = new VoiceCommandDraft(PhraseLists.NewId("custom"), phrases, action, value);
        Save(Enabled, WakePrefix, [.. Rows, row]);
    }

    /// <summary>Comma-separated phrases, trimmed and lowercased, blanks left out.</summary>
    public static IReadOnlyList<string> Phrases(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        return text.Split(',').Select(p => p.Trim().ToLowerInvariant()).Where(p => p.Length > 0).ToList();
    }

    private void Save(bool enabled, string wakePrefix, IReadOnlyList<VoiceCommandDraft> rows)
    {
        if (!Loaded)
        {
            return;
        }
        Enabled = enabled;
        WakePrefix = wakePrefix;
        Rows = rows;
        Failure = null;
        Changed();
        send(new CoreCommand.VoiceCommandsSave(enabled, wakePrefix, rows, ReplaceUnreadable: false, NextRef()));
    }

    /// <summary>What a command does, in words. Its text is shown as text, never as a link.</summary>
    public static string Describe(VoiceCommandDraft row)
    {
        ArgumentNullException.ThrowIfNull(row);
        var value = row.Value ?? "";
        return row.Action switch
        {
            CommandAction.Undo => "Undo the last dictation",
            CommandAction.ChangeStyle => $"Write in the “{value}” style",
            CommandAction.SwitchModel => $"Switch the model to {value}",
            CommandAction.TogglePolish => "Turn polish on or off",
            CommandAction.ToggleDictation => "Pause or resume dictation",
            CommandAction.OpenUrl => $"Open {value}",
            CommandAction.OpenApp => $"Open {value}",
            CommandAction.InsertText => $"Type “{value}”",
            _ => "Something this version doesn’t know",
        };
    }

    /// <summary>A row read aloud.</summary>
    public static string AccessibilityLabel(VoiceCommandDraft row)
    {
        ArgumentNullException.ThrowIfNull(row);
        return $"Voice command {string.Join(", ", row.Triggers)}: {Describe(row)}"
            + (row.Enabled ? "" : ", off")
            + (row.CarriedOut ? "" : ". Not available in this version: it types nothing");
    }

    /// <summary>Whether this screen shows the failure: every voice_commands.list and voice_commands.save.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "voice_commands.list" or "voice_commands.save";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case VoiceCommandsListed listed when latest is null || listed.Ref is null || listed.Ref == latest:
                Enabled = listed.Enabled;
                WakePrefix = listed.WakePrefix;
                Rows = listed.Commands.Select(c => new VoiceCommandDraft(c)).ToList();
                FromImport = listed.FromImport;
                Loaded = true;
                Unreadable = false;
                Failure = null;
                Changed();
                break;
            case CommandFailed failed when failed.Command == "voice_commands.list" && (failed.Id is null || failed.Id == latest):
                ReadFailed();
                break;
            case CommandFailed failed when failed.Command == "voice_commands.save":
                // The core refused to save over a stored list it cannot read: told apart by the code.
                if (failed.Code == FailureCode.ListUnreadable)
                {
                    // The stored list became unreadable after it was read: say so, and offer Start
                    // over, at once (the read below fails the same way).
                    ReadFailed();
                }
                else
                {
                    Failure = SaveFailedText;
                    Changed();
                }
                send(new CoreCommand.VoiceCommandsList(NextRef()));
                break;
            default:
                break;
        }
    }

    private void ReadFailed()
    {
        Rows = [];
        Loaded = false;
        Unreadable = true;
        Failure = LoadFailedText;
        Changed();
    }
}

/// <summary>What became of Inkwell 0.2's dictation key, said once in Settings > Voice until dismissed.</summary>
public sealed class ImportNoteModel(Action<CoreCommand> send) : ObservableModel
{
    public ImportKeyNote? Note { get; private set; }

    public void Load() => send(new CoreCommand.ImportNotes());

    /// <summary>Read: not said again.</summary>
    public void Dismiss()
    {
        Note = null;
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.ImportKeyNote, "dismissed"));
    }

    public void Apply(InkEvent e)
    {
        if (e is ImportNotes notes)
        {
            Note = notes.Key;
            Changed();
        }
    }

    /// <summary>
    /// 0.2's stored hotkey as Windows names keys (<c>super+shift+space</c> reads Win+Shift+Space);
    /// a modifier token by name (the Mac shows its symbols).
    /// </summary>
    public static string Keys(string hotkey)
    {
        ArgumentNullException.ThrowIfNull(hotkey);
        switch (hotkey)
        {
            case "fn":
                return "fn";
            case "right_cmd":
                return "Right Windows key";
            case "right_opt":
                return "Right Alt";
            case "right_ctrl":
                return "Right Ctrl";
            default:
                break;
        }
        var parts = hotkey.Split('+').Select(p => p.Trim().ToLowerInvariant()).ToList();
        if (parts.Count == 0 || parts.Any(p => p.Length == 0))
        {
            return hotkey;
        }
        return string.Join("+", parts.Select(part => part switch
        {
            "super" or "cmd" or "command" or "meta" or "win" => "Win",
            "shift" => "Shift",
            "alt" or "option" or "opt" => "Alt",
            "ctrl" or "control" => "Ctrl",
            _ => string.Concat(part[..1].ToUpperInvariant(), part[1..]),
        }));
    }

    /// <summary>The note in words, with the key dictation uses now (<paramref name="currentKey"/>, its name).</summary>
    public static string Text(ImportKeyNote note, string currentKey)
    {
        ArgumentNullException.ThrowIfNull(note);
        var old = Keys(note.Hotkey);
        var lines = new List<string>();
        switch (note.Outcome)
        {
            case ImportKeyOutcome.Combination:
                lines.Add($"Inkwell 0.2 started dictation with {old}, a key combination. Inkwell now listens for one key held on its own, so it uses {currentKey}. Pick another under Dictate if you like.");
                break;
            case ImportKeyOutcome.Replaced:
                // Fn never reaches Windows: the import set another key in its place.
                lines.Add($"Inkwell 0.2 started dictation with {old}, which never reaches Windows, so it was replaced by {currentKey}. Pick another under Dictate if you like.");
                break;
            case ImportKeyOutcome.OtherKey:
                lines.Add($"Inkwell 0.2’s dictation key ({old}) isn’t one Inkwell can listen for now, so it uses {currentKey}. Pick another under Dictate if you like.");
                break;
            default:
                break;
        }
        if (note.Toggle)
        {
            lines.Add($"Inkwell 0.2 started and stopped on separate presses. Now you hold {currentKey} while you speak and let go when you’re done.");
        }
        return string.Join(" ", lines);
    }
}
