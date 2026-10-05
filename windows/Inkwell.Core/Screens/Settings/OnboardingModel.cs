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

/// <summary>Where polish's consent step shows in the Polish step: under what asked for it.</summary>
public enum PolishStepPlace
{
    /// <summary>Under the switch (and its line).</summary>
    UnderSwitch,
    /// <summary>Under Use Groq, in the own key's disclosure.</summary>
    UnderGroqUse,
    /// <summary>Under the other providers' Use, in the own key's disclosure.</summary>
    UnderOthersUse,
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

    /// <summary>
    /// Skip, Back and Continue work: not while polish's consent step is up on the Polish step, as
    /// nothing moves behind the Mac's alert. The step's Cancel (or Escape) or its agreeing button
    /// answers it first. Off the Polish step the card can't be seen, so it holds nothing (Escape
    /// still cancels it); a step Settings asked for is its own dialog and never holds the sheet.
    /// </summary>
    public static bool CanNavigate(ConsentModel polishConsent, OnboardingStep step)
    {
        ArgumentNullException.ThrowIfNull(polishConsent);
        return step != OnboardingStep.Polish || !polishConsent.IsShowingStep(ConsentHost.Onboarding);
    }

    /// <summary>
    /// Where polish's step shows: right under what asked for it, the switch or the Use in view
    /// (Use Groq, or the other providers' Use). With the own key's disclosure closed a Use step
    /// would be hidden inside it, so it shows under the switch.
    /// </summary>
    public static PolishStepPlace PolishStepPlaceFor(bool askedBySwitch, bool ownKeyOpen, bool others) =>
        askedBySwitch || !ownKeyOpen ? PolishStepPlace.UnderSwitch
        : others ? PolishStepPlace.UnderOthersUse
        : PolishStepPlace.UnderGroqUse;

    /// <summary>
    /// Where a step asked for at <paramref name="asked"/> shows now: there while those rows show,
    /// so it never moves under a Use that did not ask; under the switch while they are hidden (the
    /// disclosure closed, or the other set of rows shown), so it is never hidden.
    /// </summary>
    public static PolishStepPlace PolishStepShownAt(PolishStepPlace asked, bool ownKeyOpen, bool others) => asked switch
    {
        PolishStepPlace.UnderGroqUse when ownKeyOpen && !others => asked,
        PolishStepPlace.UnderOthersUse when ownKeyOpen && others => asked,
        _ => PolishStepPlace.UnderSwitch,
    };

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
            ? "Inkwell writes down speech with models that run on this PC. Each is downloaded once, and only when you press Download."
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

    /// <summary>
    /// The Polish step's own key: one choice, Groq's free model, behind this disclosure, with how
    /// to get the key over its box (<see cref="GroqKeyGuide"/>, <see cref="GroqKeyGuidePlace.FirstRun"/>).
    /// </summary>
    public const string OwnKeyTitle = "Use Groq's free model";

    public const string OwnKeyBoxName = "Groq API key";

    /// <summary>The key box's placeholder: short enough to show whole.</summary>
    public const string OwnKeyPlaceholder = "Paste your Groq key";

    /// <summary>Stores the key, nothing more (CloudModel.SaveKey); Use Groq then asks and chooses.</summary>
    public const string OwnKeySave = "Save";

    /// <summary>To Settings > AI's rows, for another provider or model.</summary>
    public const string OtherProviders = "Other providers or models\u2026";

    /// <summary>Back from those rows to Groq's.</summary>
    public const string BackToGroq = "Back to Groq's free model";

    public const string ReadyTitle = "Ready";

    /// <summary>
    /// The last step's line: how to dictate, or, with no speech model installed, that one is needed
    /// (the Mac's words; never "Hold … and speak" when nothing could be typed). With no model the
    /// download follows it, then <see cref="TrayLine"/>.
    /// </summary>
    public static string ReadyLine(string keyName, bool noSpeechModel = false) => noSpeechModel
        ? "Inkwell needs a speech model before it can type what you say."
        : $"Hold {keyName}, say something, and let go. {TrayLine}";

    /// <summary>Where Inkwell lives once the sheet closes.</summary>
    public const string TrayLine = "Inkwell lives in the notification area; this window opens from there.";

    /// <summary>The ready step's warning about cards still off, or null when none is.</summary>
    public static string? StillOff(PermissionsModel permissions)
    {
        ArgumentNullException.ThrowIfNull(permissions);
        var off = permissions.OffCards;
        return off.Count == 0 ? null : $"Still off: {string.Join(", ", off.Select(c => c.Title()))}. Settings can turn them on.";
    }
}
