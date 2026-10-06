// Settings > Modes: each mode with how it writes, the apps it is picked for (each by its name and
// icon) and the language model it polishes on; and the mode editor (ModeEditorDialog), which adds,
// changes and deletes modes through the core (modes.save, modes.delete). As the Mac's ModesModel.
//
// The core is the source of truth. Every answer (modes.listed, to a list, a save or a delete) is the
// whole list as stored, sent in order, so the newest one is what shows. A save's own answer (matched
// by its ref) closes the editor; a refusal keeps the editor open with words chosen by its code, never
// the core's message. The user's words travel here (names, instructions, apps): never log them.
//
// Polish. A mode is polished only with its own switch on, Settings > AI's "Polish my words" on, a
// model to send to (the AI setting's, or the mode's own: polish_model), and the user's OK for where
// that model sends (one per destination). No "Polish" chip shows without a model; with one that
// can't be used now, a dimmed "Polish · off" says why. A mode's own model that the core does not
// hold now (missing), or that sends elsewhere than where it did when the mode was saved (moved, or
// never recorded: unrecorded), is said under the row, with Confirm… for the last two: polish never
// goes to a destination the user did not agree to for that mode. A model at a destination no
// consent covers asks for its own OK (consent.allow) before the mode is saved on it.
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

    /// <summary>Whether two identities name the same app, as the editor compares them (the core matches ignoring case).</summary>
    public static bool Same(string a, string b)
    {
        ArgumentNullException.ThrowIfNull(a);
        ArgumentNullException.ThrowIfNull(b);
        return string.Equals(a.Trim(), b.Trim(), StringComparison.OrdinalIgnoreCase);
    }

    private static string Capitalised(string word) =>
        word.Length == 0 ? word : string.Concat(word[..1].ToUpperInvariant(), word[1..]);
}

/// <summary>An app the editor offers to add: one running now, or one the user browsed to.</summary>
/// <param name="Identity">Its executable's file name, as a mode names it. Never shown.</param>
/// <param name="IconPath">Its executable, for the icon.</param>
public sealed record PickedApp(string Identity, string Name, string? IconPath);

/// <summary>The apps with a window now, for the editor's "Running now".</summary>
public interface IRunningApps
{
    /// <summary>Apps with a visible top-level window, Inkwell and Store apps' host left out, by name.</summary>
    IReadOnlyList<PickedApp> Running();
}

/// <summary>None: what tests and a build without the view use.</summary>
public sealed class NoRunningApps : IRunningApps
{
    public static NoRunningApps Instance { get; } = new();

    public IReadOnlyList<PickedApp> Running() => [];
}

/// <summary>A language model a mode can be polished on, as modes.listed lists it (polish_models).</summary>
/// <param name="Id">Its id as a mode names it (engine:&lt;id&gt;, provider:&lt;id&gt;). Never shown.</param>
/// <param name="Destination">Where it sends a dictation, as a consent names it.</param>
/// <param name="Model">The model it asks for (for the own-key provider, the one chosen in AI).</param>
/// <param name="Allowed">A polish consent covers it, and local-only mode lets it.</param>
/// <param name="BlockedLocalOnly">Local-only mode is on and it is not on this PC.</param>
public sealed record PolishModelChoice(string Id, ConsentDestination Destination, string? Model, bool Allowed, bool BlockedLocalOnly)
{
    public static PolishModelChoice From(LanguageModelChoice choice)
    {
        ArgumentNullException.ThrowIfNull(choice);
        var destination = choice.To == LlmDestination.Cloud
            ? ConsentDestination.Cloud(choice.Endpoint ?? "", choice.Name)
            : ConsentDestination.OnDevice(choice.Name);
        return new PolishModelChoice(choice.Id, destination, choice.Model, choice.Allowed, choice.BlockedLocalOnly ?? false);
    }

    /// <summary>The own-key provider: a mode may ask it for another model by name.</summary>
    public bool IsProvider => Id.StartsWith("provider:", StringComparison.Ordinal);

    /// <summary>
    /// What the editor and the rows call it: "llama3.2, on this PC", or for a cloud provider its
    /// name and the model it asks for ("Groq · llama-3.1-8b-instant"). <paramref name="modelName"/>
    /// is a mode's own model at the provider.
    /// </summary>
    public string Label(string? modelName = null)
    {
        var own = string.IsNullOrWhiteSpace(modelName) ? null : modelName;
        if (Destination.IsOnDevice)
        {
            return own is not null && IsProvider ? ConsentDestination.OnDevice(own).Label : Destination.Label;
        }
        var model = own ?? Model;
        return string.IsNullOrEmpty(model) ? Destination.Label : $"{Destination.Label} · {model}";
    }

    /// <summary>polish_model_confirm_to: where the user agreed it sends.</summary>
    internal SortedDictionary<string, object> ConfirmFields()
    {
        var fields = new SortedDictionary<string, object>(StringComparer.Ordinal) { ["to"] = Wire.Name(Destination.Kind) };
        if (!Destination.IsOnDevice)
        {
            fields["endpoint"] = Destination.Endpoint ?? "";
        }
        return fields;
    }
}

/// <summary>What a row's button does about its own model.</summary>
public enum PolishFix
{
    /// <summary>Confirm…: where the model sends now, agreed to.</summary>
    Confirm,
    /// <summary>Allow…: polish's OK for it.</summary>
    Allow,
}

/// <summary>Whether a mode's dictations are polished now, and if not, why.</summary>
public abstract record PolishState
{
    private PolishState() { }

    /// <summary>The mode's own switch is off.</summary>
    public sealed record NotWanted : PolishState;

    /// <summary>No language model at all: the AI setting has none.</summary>
    public sealed record NoModel : PolishState;

    /// <summary>The mode's own model is not held now (let go of, or another provider chosen).</summary>
    public sealed record Missing(string Name) : PolishState;

    /// <summary>The mode's own model sends elsewhere than when it was saved (Moved), or where was never recorded.</summary>
    public sealed record Confirm(PolishModelChoice Choice, string Label, bool Moved) : PolishState;

    /// <summary>Local-only mode is on, and the model is off this PC.</summary>
    public sealed record LocalOnly(PolishModelChoice Choice, string Label) : PolishState;

    /// <summary>Settings > AI's "Polish my words" is off.</summary>
    public sealed record SwitchedOff : PolishState;

    /// <summary>No polish consent covers where the model sends. Own: the mode's own model, whose OK is asked here.</summary>
    public sealed record NeedsOk(PolishModelChoice Choice, string Label, bool Own) : PolishState;

    /// <summary>Polished, on this model.</summary>
    public sealed record Ready(PolishModelChoice Choice, string Label) : PolishState;

    /// <summary>
    /// The chip: "Polish" when it runs, "Polish · off" when it is wanted and a model is there but
    /// can't be used now, and none without a model (or without the mode's switch).
    /// </summary>
    public string? Chip => this switch
    {
        NotWanted or NoModel or Missing => null,
        Ready => "Polish",
        _ => "Polish · off",
    };

    /// <summary>Whether the chip is dimmed: polish is wanted, and off.</summary>
    public bool ChipDimmed => Chip is not null && this is not Ready;

    /// <summary>Why the chip is off, for its tooltip and Narrator.</summary>
    public string? Why => this switch
    {
        Confirm => "Confirm where its model sends",
        LocalOnly => "Local only is on in AI",
        SwitchedOff => "Polish my words is off in AI",
        NeedsOk { Own: true } ok => $"Polish needs your OK to send to {ok.Choice.Destination.Label}",
        NeedsOk => "Polish needs your OK again in AI",
        _ => null,
    };

    /// <summary>The chip as Narrator reads it.</summary>
    public string? SpokenChip => Chip is null ? null : Why is null ? Chip : $"Polish, off: {Why}";

    /// <summary>What a row says under its chips about its own model, and what its button does.</summary>
    public (string Text, PolishFix? Fix)? RowNote => this switch
    {
        Missing m => ($"Its model, {m.Name}, isn't available now, so this mode isn't polished. Edit it to pick another.", null),
        Confirm { Moved: true } c => ($"Its model now sends somewhere else: {c.Label}. Confirm it to polish with it again.", PolishFix.Confirm),
        Confirm c => ($"Where its model sends was never recorded: {c.Label}. Confirm it to polish with it.", PolishFix.Confirm),
        LocalOnly l => ($"Local only is on, so nothing goes to {l.Label}. Turn Local only off in AI to use it.", null),
        NeedsOk { Own: true } ok => ($"Polish needs your OK to send this mode's words to {ok.Choice.Destination.Label}.", PolishFix.Allow),
        _ => null,
    };
}

/// <summary>One mode's row.</summary>
/// <param name="Name">Its name, as the user named it (and says it).</param>
/// <param name="Title">What the row is titled: "Everywhere else" for the default mode beside others.</param>
/// <param name="Traits">How it writes: its style, then "Clean up speech" where on.</param>
/// <param name="Polish">Whether it is polished now, and if not, why.</param>
/// <param name="Apps">The apps it is picked for, by name.</param>
public sealed record ModeRow(string Id, string Name, bool IsDefault, string Title, IReadOnlyList<string> Traits, PolishState Polish, IReadOnlyList<AppLabel> Apps)
{
    /// <summary>The apps' names in one line, or what an empty list means.</summary>
    public string AppsText => Apps.Count == 0
        ? (IsDefault ? "Every app without a mode of its own" : "No apps: used only when you switch to it by voice.")
        : string.Join(", ", Apps.Select(a => a.Name));

    /// <summary>The chips: its traits, then Polish's (if any).</summary>
    public IReadOnlyList<string> Chips => Polish.Chip is string chip ? [.. Traits, chip] : Traits;

    /// <summary>The row read aloud.</summary>
    public string AccessibilityLabel
    {
        get
        {
            var apps = Apps.Count == 0 ? (IsDefault ? "the default" : "no apps") : string.Join(", ", Apps.Select(a => a.Name));
            var chips = Polish.SpokenChip is string polish ? [.. Traits, polish] : Traits;
            return $"{Title}: {string.Join(", ", chips)}; used in {apps}";
        }
    }

    public bool Equals(ModeRow? other) =>
        other is not null && Id == other.Id && Name == other.Name && IsDefault == other.IsDefault && Title == other.Title
        && Traits.SequenceEqual(other.Traits) && Polish == other.Polish && Apps.SequenceEqual(other.Apps);

    public override int GetHashCode() => HashCode.Combine(Id, Name, IsDefault, Title, Traits.Count, Polish, Apps.Count);
}

/// <summary>
/// A save, as modes.save sends it: only the fields set are named (one left null keeps its value),
/// so an edit never overwrites what it did not change.
/// </summary>
public sealed record ModeSave
{
    /// <summary>Absent to add a mode.</summary>
    public string? Id { get; init; }
    public string? Name { get; init; }
    /// <summary>Never Other: a style this build does not know is kept by leaving it out.</summary>
    public ModeStyle? Style { get; init; }
    public bool? Polish { get; init; }
    public bool? RemoveFillers { get; init; }
    public string? PolishPrompt { get; init; }
    public IReadOnlyList<string>? Apps { get; init; }
    /// <summary>Whether the save names the mode's model (null then is the AI setting's).</summary>
    public bool SetsPolishModel { get; init; }
    public string? PolishModel { get; init; }
    /// <summary>The model at the provider (null: the one chosen in AI); sent with the model.</summary>
    public string? PolishModelName { get; init; }
    /// <summary>Confirms where the mode's model sends now: the destination the user agreed to.</summary>
    public PolishModelChoice? ConfirmTo { get; init; }
    /// <summary>Moves an app another mode has (the editor said "Moves Slack from Chat.").</summary>
    public bool TakeApps { get; init; }
    /// <summary>Replaces stored modes that can't be read (the user chose Start over).</summary>
    public bool ReplaceUnreadable { get; init; }

    /// <summary>The "mode" object.</summary>
    internal SortedDictionary<string, object> ModeFields()
    {
        var fields = new SortedDictionary<string, object>(StringComparer.Ordinal);
        if (Id is not null)
        {
            fields["id"] = Id;
        }
        if (Name is not null)
        {
            fields["name"] = Name;
        }
        if (Style is ModeStyle style && style != ModeStyle.Other)
        {
            fields["style"] = Wire.Name(style);
        }
        if (Polish is bool polish)
        {
            fields["polish"] = polish;
        }
        if (RemoveFillers is bool fillers)
        {
            fields["remove_fillers"] = fillers;
        }
        if (PolishPrompt is not null)
        {
            fields["polish_prompt"] = PolishPrompt;
        }
        if (Apps is not null)
        {
            fields["apps"] = Apps.ToList();
        }
        if (SetsPolishModel)
        {
            fields["polish_model"] = PolishModel is null ? JsonNull.Value : PolishModel;
            fields["polish_model_name"] = PolishModelName is null ? JsonNull.Value : PolishModelName;
        }
        if (ConfirmTo is not null)
        {
            fields["polish_model_confirm"] = true;
            fields["polish_model_confirm_to"] = ConfirmTo.ConfirmFields();
        }
        return fields;
    }

    public bool Equals(ModeSave? other) =>
        other is not null && Id == other.Id && Name == other.Name && Style == other.Style && Polish == other.Polish
        && RemoveFillers == other.RemoveFillers && PolishPrompt == other.PolishPrompt
        && (Apps is null ? other.Apps is null : other.Apps is not null && Apps.SequenceEqual(other.Apps))
        && SetsPolishModel == other.SetsPolishModel && PolishModel == other.PolishModel && PolishModelName == other.PolishModelName
        && ConfirmTo == other.ConfirmTo && TakeApps == other.TakeApps && ReplaceUnreadable == other.ReplaceUnreadable;

    public override int GetHashCode() => HashCode.Combine(Id, Name, Style, Polish, PolishPrompt, SetsPolishModel, PolishModel, TakeApps);
}

/// <summary>The editor's draft of one mode: what the dialog shows and changes. The model saves it.</summary>
public sealed class ModeEditor : ObservableModel
{
    private string name;
    private ModeStyle style;
    private bool removeFillers;
    private bool polish;
    private string prompt;
    private string? polishModel;
    private string polishModelName;
    private PolishModelChoice? confirmedTo;
    private string? error;
    private bool saving;
    private ConsentDestination? consentStep;
    private readonly List<string> apps;

    public ModeEditor(ModeInfo? mode, bool isDefault)
    {
        ModeId = mode?.Id;
        IsDefault = isDefault;
        Original = mode;
        name = mode?.Name ?? "";
        // A new mode writes as the built-in default does.
        style = mode?.Style ?? ModeStyle.Formal;
        removeFillers = mode?.RemoveFillers ?? true;
        polish = mode?.Polish ?? false;
        prompt = mode?.PolishPrompt ?? "";
        apps = [.. mode?.Apps ?? []];
        polishModel = mode?.PolishModel;
        polishModelName = mode?.PolishModelName ?? "";
    }

    /// <summary>The mode edited; null while adding one.</summary>
    public string? ModeId { get; }
    public bool IsDefault { get; }
    /// <summary>The mode as it was listed when the editor opened (null while adding).</summary>
    public ModeInfo? Original { get; }
    public bool Adding => ModeId is null;

    public string Name { get => name; set => Set(ref name, value); }
    public ModeStyle Style { get => style; set => Set(ref style, value); }
    public bool RemoveFillers { get => removeFillers; set => Set(ref removeFillers, value); }
    public bool Polish { get => polish; set => Set(ref polish, value); }
    /// <summary>The mode's polish instructions as typed (a TextBox's line breaks too: the core judges them); blank uses the default.</summary>
    public string Prompt { get => prompt; set => Set(ref prompt, value); }
    /// <summary>The apps it is picked for, by identity.</summary>
    public IReadOnlyList<string> Apps => apps;
    /// <summary>The mode's own model, by id; null for the AI setting's. Picking another drops a confirm.</summary>
    public string? PolishModel
    {
        get => polishModel;
        set
        {
            if (polishModel != value)
            {
                confirmedTo = null;
            }
            Set(ref polishModel, value);
        }
    }
    /// <summary>A model at the provider (provider: models only); blank for the one chosen in AI.</summary>
    public string PolishModelName { get => polishModelName; set => Set(ref polishModelName, value); }
    /// <summary>
    /// Where the user confirmed the mode's model sends now (it had moved, or was never recorded):
    /// the destination they were shown, which the save sends back as it was, never one looked up
    /// again at the save.
    /// </summary>
    public PolishModelChoice? ConfirmedTo { get => confirmedTo; internal set => Set(ref confirmedTo, value); }

    /// <summary>The user confirmed it.</summary>
    public bool Confirmed => ConfirmedTo is not null;
    /// <summary>Why the last save was refused, in words to show (null: nothing wrong).</summary>
    public string? Error { get => error; internal set => Set(ref error, value); }
    /// <summary>A save (or the OK it asked for first) is on its way.</summary>
    public bool Saving { get => saving; internal set => Set(ref saving, value); }
    /// <summary>The OK for where the mode's model sends, on screen before saving: Allow records it, then saves.</summary>
    public ConsentDestination? ConsentStep { get => consentStep; internal set => Set(ref consentStep, value); }
    /// <summary>The core refused an app another mode has: the next save moves it (that save only).</summary>
    public bool TakeApps { get; internal set; }

    /// <summary>The name changed from the one saved: voice commands that said the old one no longer do.</summary>
    public bool Renamed => Original is not null && Name.Trim() != Original.Name.Trim();

    /// <summary>The model name as saved: none for a model that is not a provider's, or when blank.</summary>
    public string? ModelNameToSend =>
        PolishModel?.StartsWith("provider:", StringComparison.Ordinal) == true && !string.IsNullOrWhiteSpace(PolishModelName)
            ? PolishModelName.Trim()
            : null;

    /// <summary>The model picked differs from the one saved (by id, or by name at the provider).</summary>
    public bool PinChanged => Original is null
        ? PolishModel is not null
        : PolishModel != Original.PolishModel || ModelNameToSend != Original.PolishModelName;

    /// <summary>"120 / 2,000", in the reader's format, counted as the core counts (Unicode scalars, as Rust's chars).</summary>
    public string PromptCount => $"{ModesModel.Count(Prompt):N0} / {ModesModel.PromptLimit:N0}";

    public bool PromptTooLong => ModesModel.Count(Prompt) > ModesModel.PromptLimit;

    internal void AddApp(string identity)
    {
        apps.Add(identity);
        Changed();
    }

    internal void RemoveApp(string identity)
    {
        apps.RemoveAll(a => AppIdentity.Same(a, identity));
        Changed();
    }

    /// <summary>For tests and the view: replaces the apps (the default mode keeps none).</summary>
    internal void SetApps(IEnumerable<string> identities)
    {
        apps.Clear();
        apps.AddRange(identities);
        Changed();
    }

    private void Set<T>(ref T field, T value)
    {
        if (!EqualityComparer<T>.Default.Equals(field, value))
        {
            field = value;
            Changed();
        }
    }
}

/// <summary>A row's Confirm… (where its model sends now) or Allow… (polish's OK for it).</summary>
/// <param name="Pin">Confirms the mode's model too (it had moved, or was never recorded); else only the OK.</param>
public sealed record ModeConfirm(string ModeId, string ModeName, PolishModelChoice Choice, string Label, bool Pin)
{
    /// <summary>The OK is asked too: no polish consent covers where it sends.</summary>
    public bool AsksOk => !Choice.Allowed && !Choice.BlockedLocalOnly;
}

public sealed class ModesModel : ObservableModel
{
    public const string FailedText = "Your modes could not be read.";
    /// <summary>The longest polish instructions the core keeps.</summary>
    public const int PromptLimit = 2000;
    public const string RefPrefix = "modes:";
    /// <summary>The default mode's row title beside other modes.</summary>
    public const string EverywhereElse = "Everywhere else";
    /// <summary>The default mode's id when the core starts over (ink-pipeline's Mode::builtin_default).</summary>
    public const string BuiltinDefaultId = "default";
    /// <summary>Inkwell's own executable: dictating into Inkwell needs no mode.</summary>
    public const string InkwellExe = "inkwell.exe";

    public const string OkFailure = "Couldn't record your OK, so nothing was saved. Try again.";
    public const string OkFailureRow = "Couldn't record your OK. Try again.";
    public const string StartOverFailure = "Couldn't start over. Try again.";

    /// <summary>A save refused after its editor was closed.</summary>
    public static string LateSaveFailure(ModeEditor editor, string words)
    {
        ArgumentNullException.ThrowIfNull(editor);
        var name = editor.Name.Trim();
        var what = editor.Adding
            ? (name.Length == 0 ? "The new mode" : $"The new mode “{name}”")
            : $"Your change to “{editor.Original?.Name ?? name}”";
        return $"{what} wasn't saved. {words}";
    }

    private readonly Action<CoreCommand> send;
    private readonly IAppDirectory apps;
    private readonly ConsentModel? consent;
    private readonly Func<bool?> polishSwitch;
    private ModesListed? listed;
    private int requests;
    /// <summary>Asked for, or listed, at least once: only then do changes elsewhere read again.</summary>
    private bool loaded;
    /// <summary>
    /// Saves waiting for the core, each with the editor that sent it: a refusal that arrives once its
    /// editor has closed is said in the section (another editor may have saved meanwhile).
    /// </summary>
    private readonly Dictionary<string, ModeEditor> pendingSaves = new(StringComparer.Ordinal);
    private (string Ref, string Name)? pendingDelete;
    private string? pendingConfirm;
    private string? pendingStartOver;
    /// <summary>An OK on its way, and what follows it once the core has recorded it.</summary>
    private (string Ref, ConsentDestination Destination, ModeConfirm? Confirm, bool SaveEditor)? pendingOk;
    private ModeEditor? editor;

    /// <param name="consent">Polish's consents: a mode's own OK is recorded there (one per destination).</param>
    /// <param name="polishSwitch">Settings > AI's "Polish my words", from the core (null until read).</param>
    public ModesModel(Action<CoreCommand> send, IAppDirectory? apps = null, IRunningApps? running = null,
        ConsentModel? consent = null, Func<bool?>? polishSwitch = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.apps = apps ?? NoInstalledApps.Instance;
        Running = running ?? NoRunningApps.Instance;
        this.consent = consent;
        this.polishSwitch = polishSwitch ?? (() => null);
    }

    public IRunningApps Running { get; }

    public IReadOnlyList<ModeRow> Rows { get; private set; } = [];

    /// <summary>The modes could not be read: nothing can be changed, until the user starts over.</summary>
    public bool Failed { get; private set; }

    /// <summary>The stored modes can't be read: Start over is offered.</summary>
    public bool Unreadable { get; private set; }

    /// <summary>What a delete, a confirm or a start-over could not do, in words (null: nothing wrong).</summary>
    public string? Problem { get; private set; }

    /// <summary>The models a mode can pick now, the AI setting's first.</summary>
    public IReadOnlyList<PolishModelChoice> Choices { get; private set; } = [];

    /// <summary>The AI setting's model (in Choices), if there is one.</summary>
    public string? SettingModel { get; private set; }

    /// <summary>The instructions a mode with blank ones uses: the editor's placeholder.</summary>
    public string DefaultPrompt { get; private set; } = "";

    /// <summary>The editor on screen.</summary>
    public ModeEditor? Editor
    {
        get => editor;
        private set
        {
            editor = value;
            Changed();
        }
    }

    /// <summary>The mode whose delete is being confirmed.</summary>
    public ModeRow? Deleting { get; private set; }

    /// <summary>A row's Confirm… or Allow…, on screen.</summary>
    public ModeConfirm? Confirming { get; private set; }

    /// <summary>Start over's confirmation, on screen.</summary>
    public bool ConfirmingStartOver { get; private set; }

    /// <summary>
    /// A row's delete, confirm or OK, or Start over, is waiting for the core: the rows' buttons wait
    /// too, so a second press never takes the first one's place.
    /// </summary>
    public bool Busy { get; private set; }

    /// <summary>A length as the core counts it: Unicode scalars, as Rust's chars.</summary>
    public static int Count(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        return text.EnumerateRunes().Count();
    }

    private string NextRef()
    {
        requests++;
        return $"{RefPrefix}{requests}";
    }

    public void Load()
    {
        loaded = true;
        send(new CoreCommand.ModesList(NextRef()));
    }

    public static string Style(ModeStyle style) => style switch
    {
        ModeStyle.Formal => "Formal",
        ModeStyle.Casual => "Casual",
        ModeStyle.Relaxed => "Relaxed",
        _ => "Own style",
    };

    /// <summary>Whether this screen shows the failure (modes.list, modes.save, modes.delete).</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "modes.list" or "modes.save" or "modes.delete";
    }

    // What the rows and the editor show.

    /// <summary>A mode's own model when the core does not hold it: named from its id, never shown as it is.</summary>
    public static string PinName(string id, string? modelName)
    {
        ArgumentNullException.ThrowIfNull(id);
        var own = string.IsNullOrWhiteSpace(modelName) ? null : modelName;
        if (id.StartsWith("provider:", StringComparison.Ordinal))
        {
            var provider = CloudModel.ProviderName(id["provider:".Length..]);
            return own is null ? provider : $"{provider} · {own}";
        }
        return "a model that was on this PC";
    }

    /// <summary>
    /// Whether a mode with these settings is polished now, and if not, why. <paramref name="ignoringSwitch"/>:
    /// as if Settings > AI's switch were on (what the editor says of the model alone).
    /// </summary>
    public PolishState StateOf(bool polish, string? pin, string? modelName, PolishModelState? pinState, bool ignoringSwitch = false)
    {
        if (!polish)
        {
            return new PolishState.NotWanted();
        }
        PolishModelChoice choice;
        string label;
        if (pin is not null)
        {
            var found = Choices.FirstOrDefault(c => c.Id == pin);
            if (pinState == PolishModelState.Missing || found is null)
            {
                return new PolishState.Missing(PinName(pin, modelName));
            }
            label = found.Label(modelName);
            if (pinState is PolishModelState.Moved or PolishModelState.Unrecorded)
            {
                return new PolishState.Confirm(found, label, pinState == PolishModelState.Moved);
            }
            choice = found;
        }
        else
        {
            var found = SettingModel is null ? null : Choices.FirstOrDefault(c => c.Id == SettingModel);
            if (found is null)
            {
                return new PolishState.NoModel();
            }
            choice = found;
            label = found.Label();
        }
        if (choice.BlockedLocalOnly)
        {
            return new PolishState.LocalOnly(choice, label);
        }
        // Unread, the switch is not called off: the core's state arrives at launch.
        if (!ignoringSwitch && polishSwitch() == false)
        {
            return new PolishState.SwitchedOff();
        }
        return choice.Allowed ? new PolishState.Ready(choice, label) : new PolishState.NeedsOk(choice, label, pin is not null);
    }

    /// <summary>The editor's draft as a take would find it once saved: a model just picked (or confirmed) is recorded where it sends now.</summary>
    public PolishState StateOf(ModeEditor editor, bool? polish = null, bool ignoringSwitch = false)
    {
        ArgumentNullException.ThrowIfNull(editor);
        var state = editor.PinChanged || Confirmed(editor) ? null : editor.Original?.PolishModelState;
        return StateOf(polish ?? editor.Polish, editor.PolishModel, editor.ModelNameToSend, state, ignoringSwitch);
    }

    /// <summary>
    /// Whether the editor's confirmation still holds: the destination confirmed is where the model
    /// sends now (a listing since may say it moved again; the save would then be refused).
    /// </summary>
    public bool Confirmed(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        return editor.ConfirmedTo is PolishModelChoice shown && Choices.FirstOrDefault(c => c.Id == shown.Id)?.Destination == shown.Destination;
    }

    /// <summary>The line under the editor's Polish switch, when the mode wants polish and nothing would polish it whatever its model.</summary>
    public string? PolishNote(ModeEditor editor) => StateOf(editor) switch
    {
        PolishState.NoModel => "No language model is available on this PC, so nothing is polished.",
        PolishState.SwitchedOff => "Polish my words is off in AI, so nothing is polished.",
        PolishState.NeedsOk { Own: false } => "Polish needs your OK again in AI, so nothing is polished.",
        _ => null,
    };

    /// <summary>What the editor says under its model: where it sends, or what stops it.</summary>
    public (string Text, bool IsProblem) ModelNote(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        // The model alone: the mode's switch and AI's are said under the Polish switch.
        var state = StateOf(editor, polish: true, ignoringSwitch: true);
        var asks = editor.Polish && (editor.PinChanged || Confirmed(editor) || !(editor.Original?.Polish ?? false));
        var prefix = editor.PolishModel is null ? "Uses the model chosen in AI. " : "";
        return state switch
        {
            PolishState.Missing => ("This model isn't available now, so this mode isn't polished. Pick another.", true),
            PolishState.Confirm { Moved: true } c => ($"Its model now sends somewhere else: {c.Label}. Confirm it to polish with it again.", true),
            PolishState.Confirm c => ($"Where its model sends was never recorded: {c.Label}. Confirm it to polish with it.", true),
            PolishState.LocalOnly => ("Local only is on, so nothing goes to it. Turn Local only off in AI first.", true),
            PolishState.NeedsOk { Own: true } ok when asks => ($"Saving asks for your OK to send this mode's words to {ok.Choice.Destination.Label}.", false),
            PolishState.NeedsOk { Own: true } ok => ($"Polish needs your OK to send this mode's words to {ok.Choice.Destination.Label}.", true),
            PolishState.NoModel => ("Uses the model chosen in AI. There is none now.", false),
            PolishState.Ready r => (prefix + Where(r.Choice), false),
            PolishState.NeedsOk ok => (prefix + Where(ok.Choice), false),
            _ => ("Uses the model chosen in AI.", false),
        };
    }

    private static string Where(PolishModelChoice choice) =>
        choice.Destination.IsOnDevice ? "Your words stay on this PC." : $"Your words go to {choice.Destination.Label}.";

    /// <summary>The editor's model picker: the AI setting's first, then each model, and a mode's own model the core no longer holds.</summary>
    public IReadOnlyList<(string? Id, string Label)> ModelOptions(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        var setting = SettingModel is null ? null : Choices.FirstOrDefault(c => c.Id == SettingModel);
        var options = new List<(string? Id, string Label)>
        {
            (null, setting is null ? "As in AI (none now)" : $"As in AI ({setting.Label()})"),
        };
        options.AddRange(Choices.Select(c => ((string?)c.Id, c.Label())));
        if (editor.PolishModel is string pin && Choices.All(c => c.Id != pin))
        {
            options.Add((pin, $"{PinName(pin, null)} (not available)"));
        }
        return options;
    }

    /// <summary>The model the own-key provider asks for unless the mode names another (the field's placeholder).</summary>
    public string? ProviderModel(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        return editor.PolishModel is string pin ? Choices.FirstOrDefault(c => c.Id == pin)?.Model : null;
    }

    /// <summary>Whether the editor offers Confirm: the mode's own model, unchanged, sends elsewhere than when it was saved, or where was never recorded.</summary>
    public bool CanConfirmInEditor(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        return !Confirmed(editor) && !editor.PinChanged && StateOf(editor, polish: true, ignoringSwitch: true) is PolishState.Confirm;
    }

    /// <summary>The editor's OK step: "Polish “Chat” with Groq?".</summary>
    public string ConsentTitle(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        var name = editor.Name.Trim();
        return StateOf(editor, ignoringSwitch: true) is PolishState.NeedsOk { Own: true } ok && name.Length > 0
            ? $"Polish “{name}” with {ok.Label}?"
            : ConsentModel.Title(LlmFeature.Polish);
    }

    /// <summary>What the OK step says: where the words go, and that it turns polish on if it is off.</summary>
    public string ConsentMessage(ConsentDestination destination)
    {
        var message = ConsentModel.Message(LlmFeature.Polish, destination);
        return polishSwitch() == false ? $"{message} This also turns on Polish my words." : message;
    }

    /// <summary>The mode other than <paramref name="modeId"/> that has <paramref name="identity"/>, by its title.</summary>
    public string? Owner(string identity, string? modeId)
    {
        ArgumentNullException.ThrowIfNull(identity);
        if (listed is null)
        {
            return null;
        }
        var mode = listed.Modes.FirstOrDefault(m => m.Id != modeId && m.Apps.Any(a => AppIdentity.Same(a, identity)));
        return mode is null ? null : mode.Id == listed.DefaultId && listed.Modes.Count > 1 ? EverywhereElse : mode.Name;
    }

    /// <summary>An app as the editor shows it.</summary>
    public AppLabel Label(string identity) => AppIdentity.Label(identity, apps);

    /// <summary>"Moves Slack from Chat." for each app the draft takes from another mode.</summary>
    public IReadOnlyList<string> MovingNotes(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        return editor.Apps
            .Where(identity => !(editor.Original?.Apps.Any(a => AppIdentity.Same(a, identity)) ?? false))
            .Select(identity => Owner(identity, editor.ModeId) is string owner ? $"Moves {Label(identity).Name} from {owner}." : null)
            .OfType<string>()
            .ToList();
    }

    /// <summary>The running apps the editor can add: not in the draft already, each with the mode it is in.</summary>
    public IReadOnlyList<(PickedApp App, string? Owner)> RunningOffers(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        return Running.Running()
            .Where(app => !editor.Apps.Any(a => AppIdentity.Same(a, app.Identity)))
            .Select(app => (app, Owner(app.Identity, editor.ModeId)))
            .ToList();
    }

    // The editor.

    /// <summary>Opens the editor on a new mode.</summary>
    public void Add()
    {
        if (!Failed)
        {
            Editor = new ModeEditor(null, isDefault: false);
        }
    }

    /// <summary>Opens the editor on a mode.</summary>
    public void Edit(string id)
    {
        if (listed?.Modes.FirstOrDefault(m => m.Id == id) is ModeInfo mode)
        {
            Editor = new ModeEditor(mode, isDefault: id == listed.DefaultId);
        }
    }

    /// <summary>
    /// Cancel, Escape or the dialog closed: nothing more is saved. A save already sent still lands;
    /// if the core refuses it, the section says so (the editor is gone). An OK still on its way is
    /// recorded, but saves nothing.
    /// </summary>
    public void CloseEditor()
    {
        if (pendingOk is { SaveEditor: true })
        {
            pendingOk = null;
        }
        Editor = null;
    }

    public static void AddApp(string identity, ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(identity);
        ArgumentNullException.ThrowIfNull(editor);
        if (editor.IsDefault || editor.Apps.Any(a => AppIdentity.Same(a, identity)))
        {
            return;
        }
        editor.AddApp(identity);
        editor.Error = null;
    }

    public static void RemoveApp(string identity, ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        editor.RemoveApp(identity);
    }

    /// <summary>An executable the user browsed to: its file name, as the core matches the app in front.</summary>
    public static void AddBrowsed(string path, ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(path);
        ArgumentNullException.ThrowIfNull(editor);
        // The file's name, whichever separator the path uses.
        var exe = path.Split('\\', '/')[^1].ToLowerInvariant();
        if (!AppIdentity.IsExe(exe))
        {
            editor.Error = "That isn't an app Inkwell can tell is in front. Pick its .exe.";
            return;
        }
        if (exe == InkwellExe)
        {
            editor.Error = "Inkwell itself needs no mode.";
            return;
        }
        AddApp(exe, editor);
    }

    /// <summary>The editor's Confirm: the user agreed to where the mode's model sends now. Saved with the mode.</summary>
    public void ConfirmInEditor(ModeEditor editor)
    {
        ArgumentNullException.ThrowIfNull(editor);
        if (!editor.PinChanged && StateOf(true, editor.PolishModel, editor.ModelNameToSend, editor.Original?.PolishModelState, ignoringSwitch: true) is PolishState.Confirm c)
        {
            editor.ConfirmedTo = c.Choice;
        }
    }

    /// <summary>
    /// Save: the mode as the editor holds it. A model newly picked (or confirmed) at a destination no
    /// polish consent covers asks for its OK first; the save follows Allow.
    /// </summary>
    public void Save()
    {
        if (Editor is not ModeEditor editor || editor.Saving)
        {
            return;
        }
        editor.Error = null;
        if (editor.Polish && (editor.PinChanged || Confirmed(editor) || !(editor.Original?.Polish ?? false))
            && StateOf(editor) is PolishState.NeedsOk { Own: true } ok)
        {
            editor.ConsentStep = ok.Choice.Destination;
            return;
        }
        SendSave(editor);
    }

    /// <summary>The editor's OK step: Allow records polish's OK for that destination, then saves.</summary>
    public void AllowAndSave(ConsentDestination destination)
    {
        if (Editor is not ModeEditor editor || editor.ConsentStep != destination || consent is null)
        {
            return;
        }
        editor.ConsentStep = null;
        editor.Saving = true;
        pendingOk = (consent.AllowForMode(destination), destination, null, true);
    }

    /// <summary>The editor's OK step's Cancel: nothing is recorded or saved; the editor stays.</summary>
    public void CancelConsentStep()
    {
        if (Editor is ModeEditor editor)
        {
            editor.ConsentStep = null;
        }
    }

    private void SendSave(ModeEditor editor, bool replaceUnreadable = false)
    {
        var original = editor.Original;
        var adding = original is null;
        var sameApps = original is not null && original.Apps.SequenceEqual(editor.Apps);
        var setsModel = editor.PinChanged || adding;
        var pin = editor.PolishModel;
        var save = new ModeSave
        {
            Id = editor.ModeId,
            // A new mode names every field; a change only what it changes.
            Name = adding || editor.Name != original!.Name ? editor.Name : null,
            Style = editor.Style == ModeStyle.Other ? null : adding || editor.Style != original!.Style ? editor.Style : null,
            Polish = adding || editor.Polish != original!.Polish ? editor.Polish : null,
            RemoveFillers = adding || editor.RemoveFillers != original!.RemoveFillers ? editor.RemoveFillers : null,
            PolishPrompt = adding || editor.Prompt != original!.PolishPrompt ? editor.Prompt : null,
            Apps = editor.IsDefault || sameApps ? null : editor.Apps.ToList(),
            SetsPolishModel = setsModel,
            PolishModel = setsModel ? pin : null,
            PolishModelName = setsModel ? editor.ModelNameToSend : null,
            // As the user was shown it: if it sends elsewhere by now, the core refuses it.
            ConfirmTo = !editor.PinChanged && editor.ConfirmedTo is PolishModelChoice shown && shown.Id == pin ? shown : null,
            TakeApps = editor.TakeApps || MovingNotes(editor).Count > 0,
            ReplaceUnreadable = replaceUnreadable,
        };
        // Said for this save only: an app someone else takes later is said again.
        editor.TakeApps = false;
        var reference = NextRef();
        pendingSaves[reference] = editor;
        editor.Saving = true;
        send(new CoreCommand.ModesSave(save, reference));
    }

    // The rows' actions.

    public void AskDelete(string id)
    {
        if (Busy)
        {
            return;
        }
        Deleting = Rows.FirstOrDefault(r => r.Id == id && !r.IsDefault);
        Changed();
    }

    public void CancelDelete()
    {
        Deleting = null;
        Changed();
    }

    /// <summary>The delete was confirmed, of <paramref name="row"/> (the one the confirmation named): its apps go back to the default mode.</summary>
    public void Delete(ModeRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        if (Deleting?.Id != row.Id || Busy)
        {
            return;
        }
        Deleting = null;
        Problem = null;
        Busy = true;
        var reference = NextRef();
        pendingDelete = (reference, row.Name);
        send(new CoreCommand.ModesDelete(row.Id, reference));
        Changed();
    }

    /// <summary>The delete confirmation's words: "Slack and Mail go back to Everywhere else."</summary>
    public static string DeleteMessage(ModeRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        var names = row.Apps.Select(a => a.Name).ToList();
        if (names.Count == 0)
        {
            return "It has no apps. Voice commands can't switch to it any more.";
        }
        var list = names.Count == 1 ? names[0] : $"{string.Join(", ", names.Take(names.Count - 1))} and {names[^1]}";
        return $"{list} {(names.Count == 1 ? "goes" : "go")} back to {EverywhereElse}.";
    }

    /// <summary>The delete confirmation's title.</summary>
    public static string DeleteTitle(ModeRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        return $"Delete “{row.Name}”?";
    }

    /// <summary>A row's Confirm… or Allow….</summary>
    public void AskConfirm(string id)
    {
        if (Busy)
        {
            return;
        }
        Problem = null;
        var row = Rows.FirstOrDefault(r => r.Id == id);
        Confirming = row?.Polish switch
        {
            PolishState.Confirm c => new ModeConfirm(id, row.Name, c.Choice, c.Label, Pin: true),
            PolishState.NeedsOk { Own: true } ok => new ModeConfirm(id, row.Name, ok.Choice, ok.Label, Pin: false),
            _ => null,
        };
        Changed();
    }

    public void CancelConfirm()
    {
        Confirming = null;
        Changed();
    }

    public static string ConfirmTitle(ModeConfirm c)
    {
        ArgumentNullException.ThrowIfNull(c);
        return $"Polish “{c.ModeName}” with {c.Label}?";
    }

    public string ConfirmMessage(ModeConfirm c)
    {
        ArgumentNullException.ThrowIfNull(c);
        return c.AsksOk ? ConsentMessage(c.Choice.Destination) : ConsentModel.Message(LlmFeature.Polish, c.Choice.Destination);
    }

    public static string ConfirmButton(ModeConfirm c)
    {
        ArgumentNullException.ThrowIfNull(c);
        return ConsentModel.Button(LlmFeature.Polish, c.Choice.Destination);
    }

    /// <summary>
    /// The confirmation's Allow: polish's OK for where the model sends, if no consent covers it,
    /// then (for a model that moved) the confirm, with the destination the user was shown.
    /// </summary>
    public void ConfirmAllow(ModeConfirm c)
    {
        ArgumentNullException.ThrowIfNull(c);
        if (Confirming != c || Busy)
        {
            return;
        }
        Confirming = null;
        Busy = true;
        if (c.AsksOk && consent is not null)
        {
            pendingOk = (consent.AllowForMode(c.Choice.Destination), c.Choice.Destination, c.Pin ? c : null, false);
        }
        else if (c.Pin)
        {
            SendConfirm(c);
        }
        else
        {
            Busy = false;
        }
        Changed();
    }

    private void SendConfirm(ModeConfirm c)
    {
        var reference = NextRef();
        pendingConfirm = reference;
        send(new CoreCommand.ModesSave(new ModeSave { Id = c.ModeId, ConfirmTo = c.Choice }, reference));
    }

    /// <summary>Start over's confirmation: its words.</summary>
    public const string StartOverTitle = "Start over?";
    public const string StartOverMessage = "Your stored modes can't be read. Start over replaces them with one default mode, Everywhere else; the modes that can't be read are gone.";

    /// <summary>Start over: asks first (it replaces the stored modes, which can't be read, with the default).</summary>
    public void AskStartOver()
    {
        if (Unreadable && !Busy)
        {
            ConfirmingStartOver = true;
            Changed();
        }
    }

    public void CancelStartOver()
    {
        ConfirmingStartOver = false;
        Changed();
    }

    /// <summary>Replaces stored modes the core can't read with the default: only when the user confirmed it.</summary>
    public void StartOver()
    {
        if (!Unreadable || !ConfirmingStartOver || Busy)
        {
            return;
        }
        ConfirmingStartOver = false;
        Busy = true;
        Problem = null;
        var reference = NextRef();
        pendingStartOver = reference;
        send(new CoreCommand.ModesSave(new ModeSave { Id = BuiltinDefaultId, ReplaceUnreadable = true }, reference));
        Changed();
    }

    // Refusals in words.

    /// <summary>What a refused save says, by its code (never the core's message: that names fields).</summary>
    public static string SaveFailure(FailureCode? code, ModeEditor? editor)
    {
        switch (code)
        {
            case FailureCode.NameBlank:
                return "Give the mode a name.";
            case FailureCode.NameTaken:
                return "Another mode has a name that sounds the same. Pick another name.";
            case FailureCode.NameIsStyle:
                return "Formal, Casual and Relaxed name the styles in voice commands. Pick another name.";
            case FailureCode.TooLong:
                // Counted as the core counts: Unicode scalars, as Rust's chars.
                if (Count(editor?.Name.Trim() ?? "") > 64)
                {
                    return "A name can be up to 64 characters.";
                }
                if (Count(editor?.Prompt ?? "") > PromptLimit)
                {
                    return "Polish instructions can be up to 2,000 characters.";
                }
                if ((editor?.Apps.Count ?? 0) > 64)
                {
                    return "Shorten to 64 apps or fewer.";
                }
                return editor?.Adding == true ? "You can have up to 50 modes." : "Something here is too long. Shorten it.";
            case FailureCode.DefaultMode:
                return $"{EverywhereElse} is used in every app without a mode of its own, so it can't be given apps.";
            case FailureCode.AppTaken:
                return "An app here is in another mode now. Save again to move it here.";
            case FailureCode.AppInvalid:
                return "One of these apps can't be told apart from others. Remove it, and pick it again.";
            case FailureCode.ModeNotFound:
                return "This mode was deleted meanwhile, so it wasn't saved.";
            case FailureCode.ModelUnknown:
                return "That model isn't available any more. Pick another.";
            case FailureCode.ModelNameInvalid:
                return "A model name can be up to 128 characters, on one line.";
            case FailureCode.DestinationChanged:
                return "Where its model sends changed while you looked. Check it, and confirm again.";
            case FailureCode.ListUnreadable:
                return "Your modes can't be read, so this wasn't saved. Start over replaces them with the default.";
            default:
                return "Couldn't save the mode. Try again.";
        }
    }

    /// <summary>What a refused delete says.</summary>
    public static string DeleteFailure(FailureCode? code, string name) => code switch
    {
        FailureCode.DefaultMode => $"{EverywhereElse} can't be deleted: it is used in every app without a mode of its own.",
        FailureCode.ModeNotFound => $"“{name}” was already deleted.",
        FailureCode.ListUnreadable => $"Your modes can't be read, so “{name}” wasn't deleted. Start over replaces them with the default.",
        _ => $"Couldn't delete “{name}”. Try again.",
    };

    /// <summary>What a refused confirm says.</summary>
    public static string ConfirmFailure(FailureCode? code) => code switch
    {
        FailureCode.DestinationChanged => "Where its model sends changed while you looked. Check it, and confirm again.",
        FailureCode.ModeNotFound => "That mode was deleted meanwhile.",
        FailureCode.ModelUnknown => "Its model isn't available any more. Edit the mode to pick another.",
        FailureCode.ListUnreadable => "Your modes can't be read, so nothing was confirmed. Start over replaces them with the default.",
        _ => "Couldn't confirm its model. Try again.",
    };

    // Events.

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case ModesListed value:
                Show(value);
                if (value.Ref is string reference)
                {
                    if (pendingSaves.Remove(reference, out var sender))
                    {
                        // Saved: the editor that sent it closes (unless it already did).
                        if (ReferenceEquals(editor, sender))
                        {
                            editor = null;
                        }
                    }
                    else if (reference == pendingDelete?.Ref)
                    {
                        pendingDelete = null;
                        Finished();
                    }
                    else if (reference == pendingConfirm)
                    {
                        pendingConfirm = null;
                        Finished();
                    }
                    else if (reference == pendingStartOver)
                    {
                        pendingStartOver = null;
                        Finished();
                    }
                }
                Changed();
                break;
            case CommandFailed failed when failed.Command == "modes.list":
                // Not known what is stored: nothing is shown, and nothing can be changed. Try again
                // reads again; Start over (which asks first) replaces modes the core can't read, and
                // changes nothing over ones it can.
                Failed = true;
                Unreadable = true;
                Rows = [];
                listed = null;
                Changed();
                break;
            case CommandFailed failed when failed.Command is "modes.save" or "modes.delete":
                Refused(failed);
                break;
            case CommandFailed failed when failed.Command == "consent.allow" && pendingOk is not null && failed.Id == pendingOk.Value.Ref:
                OkFailed();
                break;
            case ConsentState state when state.Feature == LlmFeature.Polish:
                if (pendingOk is { } pending && state.Ref == pending.Ref)
                {
                    if (ConsentSnapshot.From(state).Covers(pending.Destination))
                    {
                        pendingOk = null;
                        if (pending.SaveEditor && Editor is ModeEditor open)
                        {
                            SendSave(open);
                        }
                        else if (pending.Confirm is ModeConfirm c)
                        {
                            SendConfirm(c);
                        }
                        else if (!pending.SaveEditor)
                        {
                            // The OK was all the row asked for.
                            Finished();
                            Changed();
                        }
                    }
                    else
                    {
                        // The model sends elsewhere now, or the store refused: nothing was recorded.
                        OkFailed();
                    }
                }
                // Which model each mode may use now.
                Reload();
                break;
            case CoreStopped:
                // Nothing in flight will be answered: the buttons are back, and nothing waits.
                foreach (var waiting in pendingSaves.Values)
                {
                    waiting.Saving = false;
                }
                pendingSaves.Clear();
                pendingDelete = null;
                pendingConfirm = null;
                pendingStartOver = null;
                pendingOk = null;
                Busy = false;
                Changed();
                break;
            case LlmProviders:
            case EngineUnregistered:
            case EngineRegistered { Kind: EngineKind.Llm }:
            case SettingValue { Key: "llm.local_only" }:
            // A take found a mode's model gone or moved: show which.
            case DictationWarningEvent { Kind: DictationWarning.PolishModelMissing }:
                Reload();
                break;
            default:
                break;
        }
    }

    /// <summary>Reads the modes again after a change elsewhere, once they have been read.</summary>
    private void Reload()
    {
        if (loaded)
        {
            send(new CoreCommand.ModesList(NextRef()));
        }
    }

    /// <summary>A row's operation has its answer: a stale failure goes, and the buttons are back.</summary>
    private void Finished()
    {
        Busy = false;
        Problem = null;
    }

    private void OkFailed()
    {
        if (pendingOk is not { } pending)
        {
            return;
        }
        pendingOk = null;
        if (pending.SaveEditor)
        {
            if (Editor is ModeEditor open)
            {
                open.Saving = false;
                open.Error = OkFailure;
            }
        }
        else
        {
            Busy = false;
            Problem = OkFailureRow;
        }
        Changed();
    }

    private void Refused(CommandFailed failed)
    {
        if (failed.Id is not string id)
        {
            return;
        }
        if (failed.Code == FailureCode.ListUnreadable)
        {
            Unreadable = true;
        }
        if (pendingSaves.Remove(id, out var sender))
        {
            sender.Saving = false;
            if (failed.Code == FailureCode.AppTaken)
            {
                sender.TakeApps = true;
            }
            if (failed.Code is FailureCode.DestinationChanged or FailureCode.ModelUnknown)
            {
                // What was confirmed is not where the model sends now: Confirm asks again.
                sender.ConfirmedTo = null;
            }
            var words = SaveFailure(failed.Code, sender);
            if (ReferenceEquals(Editor, sender))
            {
                sender.Error = words;
            }
            else
            {
                // Cancelled while the save was on its way: said where the user is now.
                Problem = LateSaveFailure(sender, words);
            }
        }
        else if (pendingDelete is { } pending && id == pending.Ref)
        {
            pendingDelete = null;
            Busy = false;
            Problem = DeleteFailure(failed.Code, pending.Name);
        }
        else if (id == pendingConfirm)
        {
            pendingConfirm = null;
            Busy = false;
            Problem = ConfirmFailure(failed.Code);
        }
        else if (id == pendingStartOver)
        {
            pendingStartOver = null;
            Busy = false;
            Problem = StartOverFailure;
            Changed();
            return;
        }
        else
        {
            return;
        }
        Changed();
        // What the refusal says may have changed: show the modes as stored now.
        if (failed.Code is FailureCode.ModeNotFound or FailureCode.ModelUnknown or FailureCode.DestinationChanged or FailureCode.AppTaken)
        {
            Reload();
        }
    }

    private void Show(ModesListed value)
    {
        listed = value;
        loaded = true;
        Failed = false;
        Unreadable = false;
        Choices = value.PolishModels.Select(PolishModelChoice.From).ToList();
        SettingModel = value.SettingPolishModel;
        DefaultPrompt = value.DefaultPolishPrompt;
        // The default mode last, as "everywhere else": the others are matched first.
        var others = value.Modes.Where(m => m.Id != value.DefaultId);
        var fallback = value.Modes.Where(m => m.Id == value.DefaultId);
        Rows = others.Concat(fallback).Select(mode => Row(mode, value)).ToList();
    }

    private ModeRow Row(ModeInfo mode, ModesListed value)
    {
        var traits = new List<string> { Style(mode.Style) };
        if (mode.RemoveFillers)
        {
            traits.Add("Clean up speech");
        }
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var labels = mode.Apps
            .Where(a => !string.IsNullOrWhiteSpace(a))
            .Select(a => AppIdentity.Label(a, apps))
            .Where(label => seen.Add(label.Id))
            .ToList();
        var isDefault = mode.Id == value.DefaultId;
        var title = isDefault && value.Modes.Count > 1 ? EverywhereElse : mode.Name;
        return new ModeRow(mode.Id, mode.Name, isDefault, title, traits,
            StateOf(mode.Polish, mode.PolishModel, mode.PolishModelName, mode.PolishModelState), labels);
    }
}
