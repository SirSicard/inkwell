// Settings > AI's summaries-and-Ask switch and Settings > Voice's edit-key picker: the parts of the
// Mac's ScreenModels that join the consent models to polish's engine test and the dictation keys.
//
// Summaries and Ask read on only with a consent that covers the model now and a working language
// model; their switch only asks (the consent step), and off sends setting.set meetings.llm off,
// which withdraws the consent in the same write. Voice edit has a consent of its own, turned on by
// picking its key (off by default): a key while voice edit is on and allowed only changes the key;
// otherwise the step asks first, and Allow sends the key with the consent.
//
// This model holds no state of its own: the consent models are fed by the aggregator (Apply on
// each), and a change in any model it reads is passed on to its view.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class AiSettings : ObservableModel
{
    private readonly Action<CoreCommand> send;

    /// <param name="editConsent">Voice edit's consent (<see cref="NewEditConsent"/>).</param>
    /// <param name="meetingsConsent">Summaries and Ask's consent (<see cref="NewMeetingsConsent"/>).</param>
    public AiSettings(
        PolishModel polish, DictationModel dictation, ConsentModel editConsent, ConsentModel meetingsConsent, Action<CoreCommand> send)
    {
        ArgumentNullException.ThrowIfNull(polish);
        ArgumentNullException.ThrowIfNull(dictation);
        ArgumentNullException.ThrowIfNull(editConsent);
        ArgumentNullException.ThrowIfNull(meetingsConsent);
        ArgumentNullException.ThrowIfNull(send);
        if (editConsent.Feature != LlmFeature.Edit || meetingsConsent.Feature != LlmFeature.Meetings)
        {
            throw new ArgumentException("the edit and meetings consents, in that order");
        }
        Polish = polish;
        Dictation = dictation;
        EditConsent = editConsent;
        MeetingsConsent = meetingsConsent;
        this.send = send;
        polish.PropertyChanged += (_, _) => Changed();
        dictation.PropertyChanged += (_, _) => Changed();
        editConsent.PropertyChanged += (_, _) => Changed();
        meetingsConsent.PropertyChanged += (_, _) => Changed();
    }

    /// <summary>The id of the summaries switch's command (a command.failed carries it).</summary>
    public static string MeetingsAISettingId => ShellSetting.MeetingsLlm.CommandId();

    /// <summary>Voice edit's consent: its switch is the edit key.</summary>
    public static ConsentModel NewEditConsent(Action<CoreCommand> send) =>
        new(LlmFeature.Edit, DictationModel.EditKeySettingId, send);

    /// <summary>The consent and switch for a meeting's summary and Ask.</summary>
    public static ConsentModel NewMeetingsConsent(Action<CoreCommand> send) =>
        new(LlmFeature.Meetings, MeetingsAISettingId, send);

    public PolishModel Polish { get; }

    public DictationModel Dictation { get; }

    public ConsentModel EditConsent { get; }

    public ConsentModel MeetingsConsent { get; }

    /// <summary>
    /// The Voice section's edit picker. Off turns voice edit off (the core withdraws its consent in
    /// the same write). A key while voice edit is on and allowed only changes the key; otherwise
    /// the consent step asks first (Allow sends the key with the consent; Cancel changes nothing).
    /// </summary>
    public void ChooseEditKey(string? token)
    {
        if (token is null)
        {
            EditConsent.SwitchedOff();
            Dictation.SetEditKey(null);
            return;
        }
        if (Dictation.EditKey is not null && EditConsent.IsAllowedOn)
        {
            Dictation.SetEditKey(token);
        }
        else
        {
            EditConsent.Ask(ConsentHost.Settings, token);
        }
    }

    /// <summary>What the Voice section says under the keys about voice edit's consent, if anything.</summary>
    public string? EditConsentProblem => EditConsent.Problem;

    // Voice edit's switch (Settings > AI, Windows): the same consent as the edit-key picker, as a
    // switch beside polish and summaries. On picks the first key the picker offers (never the
    // dictation key) and asks first; off is the picker's Off.

    /// <summary>The switch reads on only with an edit key, a consent that covers the model now, and a working model.</summary>
    public bool EditOn => Dictation.EditKey is not null && EditConsent.IsAllowedOn && Polish.HasWorkingEngine;

    /// <summary>Whether the switch can be used: a working model, and the core has said where it sends.</summary>
    public bool CanToggleEdit =>
        Polish.HasWorkingEngine && EditConsent.State is not null && EditConsent.Destination is not null;

    /// <summary>The line under the switch. A failure or an unread state is said first, never read as off.</summary>
    public string EditStatus
    {
        get
        {
            if ((EditConsent.Failure is not null || EditConsent.State?.Error is not null) && EditConsent.Problem is string failed)
            {
                return failed;
            }
            if (!Polish.HasWorkingEngine)
            {
                return "No language model is available on this PC, so voice edit stays off.";
            }
            if (EditConsent.Problem is string problem)
            {
                return problem;
            }
            if (!EditOn || Dictation.EditKey is not string key || EditConsent.Destination is not ConsentDestination destination)
            {
                return "Off. Nothing you select is sent anywhere.";
            }
            var cap = DictationModel.Cap(key);
            return destination.IsOnDevice
                ? $"On: select text, hold {cap}, say what to change. It stays on this PC."
                : $"On: select text, hold {cap}, say what to change. The selection and what you say go to {destination.Label}.";
        }
    }

    /// <summary>Whether the status is a problem to show in the alert colour.</summary>
    public bool EditIsProblem => EditConsent.IsProblem;

    /// <summary>
    /// The user switched voice edit. On asks first (the consent step, with the first key the edit
    /// picker offers); off turns it off, and the core withdraws the consent in the same write.
    /// </summary>
    public void SetEdit(bool on)
    {
        if (!CanToggleEdit)
        {
            return;
        }
        if (!on)
        {
            ChooseEditKey(null);
            return;
        }
        var offered = Dictation.EditKeys;
        var key = Dictation.EditKey ?? (offered.Count > 0 ? offered[0].Token : null);
        if (key is not null)
        {
            EditConsent.Ask(ConsentHost.Settings, key);
        }
    }

    /// <summary>Summaries and Ask's switch reads on only with a consent that covers the model now and a working model.</summary>
    public bool MeetingsAIOn => MeetingsConsent.IsAllowedOn && Polish.HasWorkingEngine;

    /// <summary>Whether their switch can be used: a working model, and the core has said where it sends.</summary>
    public bool CanToggleMeetingsAI =>
        Polish.HasWorkingEngine && MeetingsConsent.State is not null && MeetingsConsent.Destination is not null;

    /// <summary>The line under their switch. A failure or an unread state is said first, never read as off.</summary>
    public string MeetingsAIStatus
    {
        get
        {
            if ((MeetingsConsent.Failure is not null || MeetingsConsent.State?.Error is not null) && MeetingsConsent.Problem is string failed)
            {
                return failed;
            }
            if (!Polish.HasWorkingEngine)
            {
                return "No language model is available on this PC, so meetings get no summary.";
            }
            if (MeetingsConsent.Problem is string problem)
            {
                return problem;
            }
            if (!MeetingsAIOn || MeetingsConsent.Destination is not ConsentDestination destination)
            {
                return "Off. Meetings are recorded and transcribed, with no summary, and Ask stays off.";
            }
            return destination.IsOnDevice
                ? $"On, with {destination.Label}. Meeting transcripts stay on this PC."
                : $"On. Meeting transcripts go to {destination.Label} for summaries and Ask.";
        }
    }

    /// <summary>Whether the status is a problem to show in the alert colour.</summary>
    public bool MeetingsAIIsProblem => MeetingsConsent.IsProblem;

    /// <summary>
    /// The user switched summaries and Ask. On shows the consent step (nothing is sent until
    /// Allow); off turns them off, and the core withdraws the consent in the same write.
    /// </summary>
    public void SetMeetingsAI(bool on)
    {
        if (!CanToggleMeetingsAI)
        {
            return;
        }
        if (on)
        {
            MeetingsConsent.Ask(ConsentHost.Settings);
            return;
        }
        MeetingsConsent.SwitchedOff();
        send(new CoreCommand.SettingSet(ShellSetting.MeetingsLlm, "off"));
    }

    /// <summary>What a record without a summary says while summaries are off (null: on, or not read yet).</summary>
    public string? SummaryOffNote =>
        MeetingsConsent.State is null || MeetingsConsent.IsAllowedOn
            ? null
            : "Summaries are off until you allow them in Settings > AI. Meetings are still recorded and transcribed.";

    /// <summary>Whether Settings shows <paramref name="failed"/>: voice edit's and summaries' consent commands, and their switches.</summary>
    public bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return EditConsent.Handles(failed) || MeetingsConsent.Handles(failed);
    }
}
