// The first-run state: a sheet over the window until the user finishes or skips it, remembered in
// the core's store (onboarding.done). What Inkwell does, the four permission cards (nothing asked
// for until the user presses a card's button), the models as choices by outcome (ModelChoices:
// nothing downloads until the user presses the step's Download, which carries the total; the
// downloads are the CatalogueModel's and go on after the sheet), Inkwell 0.2's history (only while
// there is some to import: Import02Model.Offered), the appearance (the mode and the dots, which
// Settings > Appearance holds too), polish (off, and turned on only through its
// consent step: the sheet's switch calls PolishModel.SetOn(on, ConsentHost.Onboarding), which only
// asks), and how to dictate. A port of the Mac's OnboardingModel and OnboardingView's words.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public enum OnboardingStep
{
    Welcome,
    Permissions,
    /// <summary>What Inkwell should be able to do, as choices, and one Download for them (ModelChoices).</summary>
    Models,
    /// <summary>Only while Inkwell 0.2's data is offered.</summary>
    ImportData,
    /// <summary>Light, dark or the system's, and the dots (AppearanceModel).</summary>
    Appearance,
    Polish,
    Ready,
}

public sealed class OnboardingModel : ObservableModel
{
    private static readonly OnboardingStep[] Steps = Enum.GetValues<OnboardingStep>();

    private readonly Action<CoreCommand> send;
    private readonly ScreenLog log;
    private readonly Import02Model? import;

    /// <param name="import">Inkwell 0.2's import, whose step shows while it is offered (null: never).</param>
    public OnboardingModel(Action<CoreCommand> send, ScreenLog? log = null, Import02Model? import = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.log = log ?? ScreenLog.System;
        this.import = import;
    }

    /// <summary>The steps shown, in order.</summary>
    public IReadOnlyList<OnboardingStep> ShownSteps =>
        Steps.Where(s => s != OnboardingStep.ImportData || import?.Offered == true).ToList();

    /// <summary>The id of this model's setting commands.</summary>
    public static string SettingId => ShellSetting.OnboardingDone.CommandId();

    /// <summary>Null until the store answers; then whether it was completed.</summary>
    public bool? Completed { get; private set; }

    /// <summary>The app is quitting: the sheet is ended, and nothing is recorded.</summary>
    public bool Quitting { get; private set; }

    public OnboardingStep Step { get; private set; } = OnboardingStep.Welcome;

    /// <summary>Whether the window shows it.</summary>
    public bool Showing => Completed == false && !Quitting;

    public void Load() => send(new CoreCommand.SettingGet(ShellSetting.OnboardingDone));

    public void Next()
    {
        var following = ShownSteps.Where(s => s > Step).ToList();
        if (following.Count == 0)
        {
            Finish();
            return;
        }
        Step = following[0];
        Changed();
    }

    public void Back()
    {
        var previous = ShownSteps.Where(s => s < Step).ToList();
        if (previous.Count == 0)
        {
            return;
        }
        Step = previous[^1];
        Changed();
    }

    /// <summary>The app is quitting: the sheet goes, and the first run stays not completed, so the next launch shows it.</summary>
    public void AppQuitting()
    {
        Quitting = true;
        Changed();
    }

    /// <summary>The sheet went away without Start or Skip (Escape): skipped, unless the app is quitting.</summary>
    public void SheetDismissed()
    {
        if (Showing)
        {
            Finish();
        }
    }

    /// <summary>Done or skipped: not shown again.</summary>
    public void Finish()
    {
        Completed = true;
        send(new CoreCommand.SettingSet(ShellSetting.OnboardingDone, "true"));
        Changed();
    }

    /// <summary>
    /// Whether this model shows <paramref name="failed"/>: its read (it shows the sheet). A write
    /// that failed is not shown (the first run shows again next launch), so it is logged.
    /// </summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command == "setting.get" && failed.Id == SettingId;
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case SettingValue value when value.Key == ShellSetting.OnboardingDone.Key():
                Completed = value.Value == "true";
                Changed();
                break;
            case CommandFailed { Command: "setting.get" } failed when failed.Id == SettingId:
                // Not known whether it was completed: show it rather than never show it. Completing it
                // again costs a click; a first run that never appears costs the permissions.
                log.Write("setting.get for onboarding.done failed; showing the first run");
                if (Completed is null)
                {
                    Completed = false;
                    Changed();
                }
                break;
            default:
                break;
        }
    }

    // The sheet's words and buttons.

    /// <summary>Where the user is, for the step dots' accessible name.</summary>
    public string StepLabel
    {
        get
        {
            var shown = ShownSteps.ToList();
            return $"Step {Math.Max(0, shown.IndexOf(Step)) + 1} of {shown.Count}";
        }
    }

    public bool ShowsSkip => Step != OnboardingStep.Ready;

    public bool ShowsBack => Step != OnboardingStep.Welcome;

    /// <summary>Start on the last step; on the import step, Not now until something came over.</summary>
    public string NextTitle => Step switch
    {
        OnboardingStep.Ready => "Start",
        OnboardingStep.ImportData when import?.Imported is null => Import02Model.NotNow,
        _ => "Continue",
    };

    public const string SkipHint = "Closes this; Settings has everything here";

    /// <summary>The welcome step's lines; <paramref name="keyName"/> is the dictation key's name (DictationModel.Key(token).Name).</summary>
    public static IReadOnlyList<string> WelcomeLines(string keyName) =>
    [
        $"Hold {keyName} and speak: your words are typed where your cursor is.",
        "In a meeting, Inkwell writes down both sides as they talk, then blots the transcript and lists what you promised.",
        "It all happens on this PC. Nothing is sent anywhere unless you add your own key for a model online.",
    ];

    public const string PermissionsTitle = "What Inkwell needs";

    public const string PermissionsNote = "Nothing is asked for until you press a card's button, and each can be changed later in Settings.";

    public const string ModelsTitle = "Models";

    /// <summary>The step's choices: which are ticked (the first is always).</summary>
    public ModelChoices Choices { get; } = new();

    /// <summary>Shown while a download runs: Continue does not wait for it.</summary>
    public const string ModelsGoOn = "You can go on: the downloads continue, and Settings > Models shows them.";

    /// <summary>Asks for the model list again when it could not be read.</summary>
    public const string ModelsTryAgain = "Try again";

    /// <summary>The line over the models step's choices: what they are for, or where the list is.</summary>
    public static string ModelsNote(CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        if (catalogue.Failed)
        {
            return CatalogueModel.FailedText;
        }
        if (!catalogue.Listed)
        {
            return "Checking which models are on this PC…";
        }
        return ModelChoices.Shown(catalogue).Any(c => !ModelChoices.Installed(c, catalogue))
            ? "Inkwell turns speech into text with models that run on this PC. Choose what it should do; you can add the rest later in Settings > Models."
            : "Every model Inkwell uses is on this PC.";
    }

    public const string AppearanceTitle = "Appearance";

    public const string AppearanceNote = "You can change this and pick your own colours in Settings > Appearance.";

    public const string PolishTitle = "Polish";

    public const string PolishNote =
        "Polish tidies a dictation's wording before it is typed. It sends what you dictate to a language model, so it stays off unless you turn it on here or in Settings.";

    public const string PolishToggle = "Polish my words";

    /// <summary>The own-key provider the Polish step offers while this PC has no language model: Groq, for its free tier.</summary>
    public const string OwnKeyProvider = "groq";

    /// <summary>Where Groq's keys are made (the homepage's link).</summary>
    public const string OwnKeyUrl = "https://console.groq.com";

    /// <summary>The Polish step's own key, while no language model is available: one choice, the link in it.</summary>
    public const string OwnKeyLine = "Use Groq's free model: get a key at console.groq.com";

    /// <summary>The part of <see cref="OwnKeyLine"/> that is the link to <see cref="OwnKeyUrl"/>.</summary>
    public const string OwnKeyHost = "console.groq.com";

    /// <summary>What Save means, said before it is pressed.</summary>
    public const string OwnKeyNote =
        "Groq's free tier needs no credit card, and shows a new key once: copy it there and paste it here. Saving turns local-only mode off, so polish can send to Groq once you turn it on and allow it. The key is kept in Windows Credential Manager, never in Inkwell's files; Settings > AI has the other providers.";

    /// <summary>Stores the key and chooses Groq (CloudModel.UseKey).</summary>
    public const string OwnKeyButton = "Save";

    public const string OwnKeyBoxName = "Groq API key";

    /// <summary>The key box's placeholder: short enough to show whole.</summary>
    public const string OwnKeyPlaceholder = "Paste your Groq key";

    /// <summary>
    /// Said over the key box when a Groq key is already stored: it is the Windows account's (every
    /// Inkwell on it shares Credential Manager's entry), so it shows here in any library. Null
    /// when none is stored.
    /// </summary>
    public static string? StoredKeyLine(CloudModel cloud)
    {
        ArgumentNullException.ThrowIfNull(cloud);
        return cloud.Providers.Any(p => p.Id == OwnKeyProvider && p.HasKey)
            ? "A Groq key is already stored for this Windows account, in Windows Credential Manager: every Inkwell on this account can use it. Saving a new one replaces it."
            : null;
    }

    public const string ReadyTitle = "Ready";

    public static string ReadyLine(string keyName) =>
        $"Hold {keyName}, say something, and let go. Inkwell lives in the notification area; this window opens from there.";

    /// <summary>The ready step's warning about cards still off, or null when none is.</summary>
    public static string? StillOff(PermissionsModel permissions)
    {
        ArgumentNullException.ThrowIfNull(permissions);
        var off = permissions.OffCards;
        return off.Count == 0 ? null : $"Still off: {string.Join(", ", off.Select(c => c.Title()))}. Settings can turn them on.";
    }
}
