// The user's consent for one feature that sends their words to a language model: polish (the
// dictation), voice edit (the selection and the instruction), or summaries and Ask (a meeting's
// transcript). Each has its own. A port of the Mac's ConsentModel.
//
// The feature is off until the user turns it on through a consent step that says where the words
// go: a model on this PC (they stay on it) or a named cloud provider (they leave it). Turning it on
// only asks (Pending); Allow sends consent.allow naming what was shown (and, for voice edit, the
// key), and the core records the consent and turns the feature on in one write. Cancel sends
// nothing. The core is the source of truth: consent.state says the switch, where the feature would
// send now, and whether the consent covers that. When the model moves elsewhere the consent no
// longer covers it: the feature is paused, and turning it on asks again. The core refuses to send
// meanwhile, so nothing depends on this screen being open.
//
// Each request carries a ref of its own ("consent.get:<feature>:<n>"), and only the answer to the
// newest request is applied: an older answer that arrives late never overwrites a newer state.
// That is also why engine churn (every engine.registered or engine.unregistered reads the state
// again) needs no debounce: each read is a short store query, and only the last answer counts.
//
// Windows: there is no Apple Intelligence. The on-device model's name comes from the core
// (consent.state's name); without one the step says "the on-device model on this PC".
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>Where a feature sends the user's words, as the consent step names it.</summary>
/// <param name="Kind">On this PC, or a cloud provider.</param>
/// <param name="Endpoint">For a cloud model, the endpoint consent.state named (consent.allow sends it back).</param>
/// <param name="Name">The model's name as the core reports it (for a cloud model, its provider's name).</param>
public sealed record ConsentDestination(LlmDestination Kind, string? Endpoint, string Name)
{
    public static ConsentDestination OnDevice(string name) => new(LlmDestination.OnDevice, null, name);

    public static ConsentDestination Cloud(string endpoint, string name) => new(LlmDestination.Cloud, endpoint, name);

    public bool IsOnDevice => Kind == LlmDestination.OnDevice;

    /// <summary>The words for it: the model on this PC, or the provider's name.</summary>
    public string Label => Kind switch
    {
        LlmDestination.OnDevice => Name.Length == 0 ? "the on-device model on this PC" : $"{Name}, on this PC",
        _ => Name.Length == 0 ? "a cloud provider" : Name,
    };
}

/// <summary>The core's state for a feature, as the consent model keeps it.</summary>
/// <param name="On">The feature's switch.</param>
/// <param name="Allowed">Whether the consent covers the model the feature would use now.</param>
/// <param name="Destination">Where the feature would send now; null with no model.</param>
/// <param name="Error">What the core could not read ("couldn't read ...").</param>
public sealed record ConsentSnapshot(bool On, bool Allowed, ConsentDestination? Destination, string? Error)
{
    public static ConsentSnapshot From(ConsentState state)
    {
        ArgumentNullException.ThrowIfNull(state);
        var destination = state.To switch
        {
            LlmDestination.OnDevice => ConsentDestination.OnDevice(state.Name ?? ""),
            LlmDestination.Cloud when state.Endpoint is not null => ConsentDestination.Cloud(state.Endpoint, state.Name ?? ""),
            _ => null,
        };
        return new ConsentSnapshot(state.On, state.Allowed, destination, state.Error);
    }
}

/// <summary>The screens that can turn a feature on.</summary>
public enum ConsentHost
{
    Settings,
    Onboarding,
}

public enum ConsentFailure
{
    /// <summary>The state could not be read.</summary>
    Read,
    /// <summary>The switch-off could not be saved.</summary>
    Write,
    /// <summary>
    /// The consent could not be recorded (the model changed while the user read, or the store
    /// refused): the feature stays as it was.
    /// </summary>
    Allow,
}

public sealed class ConsentModel : ObservableModel
{
    private readonly Action<CoreCommand> send;
    /// <summary>The state before a switch-off the core has not confirmed, to put back if saving fails.</summary>
    private ConsentSnapshot? beforeOff;
    private bool hasBeforeOff;
    /// <summary>Requests sent so far, for their refs.</summary>
    private int requests;
    /// <summary>The newest request of either kind: only its answer (or an unsolicited state) is applied.</summary>
    private string? newestRef;
    /// <summary>The newest consent.get, and the newest consent.allow, for their failures.</summary>
    private string? newestGet;
    private string? newestAllow;

    /// <param name="switchSettingId">The id of the command that turns the feature's switch off (its command.failed carries it).</param>
    public ConsentModel(LlmFeature feature, string switchSettingId, Action<CoreCommand> send)
    {
        ArgumentNullException.ThrowIfNull(send);
        Feature = feature;
        SwitchSettingId = switchSettingId;
        this.send = send;
    }

    public LlmFeature Feature { get; }

    public string SwitchSettingId { get; }

    /// <summary>The core's state (null until read).</summary>
    public ConsentSnapshot? State { get; private set; }

    /// <summary>Something could not be read, recorded or saved.</summary>
    public ConsentFailure? Failure { get; private set; }

    /// <summary>The consent step on screen: where the feature would send, waiting for Allow or Cancel.</summary>
    public ConsentDestination? Pending { get; private set; }

    /// <summary>Which screen asked, so only that one shows the step (the first-run sheet can sit over Settings).</summary>
    public ConsentHost? Host { get; private set; }

    /// <summary>
    /// How many steps were put on screen. The first-run sheet shows the step inside it, under the
    /// own key's rows, so it brings each new one into view (a second Use asks again with the same
    /// words, and counts), and never scrolls for anything else.
    /// </summary>
    public int Asked { get; private set; }

    /// <summary>For voice edit, the key the step turns it on with.</summary>
    public string? PendingKey { get; private set; }

    /// <summary>The step on screen names a model that is not chosen yet: Allow chooses it first.</summary>
    public bool Choosing { get; private set; }

    /// <summary>
    /// The destination the user allowed before its model was chosen, waiting for the core to name
    /// it; the consent is sent then.
    /// </summary>
    public ConsentDestination? Agreed { get; private set; }

    /// <summary>What Allow does first when the step names a model not chosen yet; whether it sent the choice.</summary>
    private Func<bool>? choose;

    /// <summary>Where the feature would send now, if the core named a model.</summary>
    public ConsentDestination? Destination => State?.Destination;

    /// <summary>On, and the consent covers the model now.</summary>
    public bool IsAllowedOn => State is { On: true, Allowed: true };

    /// <summary>On, but the model now sends somewhere the user has not agreed to: nothing is sent.</summary>
    public bool IsPaused => State is { On: true, Allowed: false } && Destination is not null;

    /// <summary>A problem to show in the alert colour.</summary>
    public bool IsProblem => Failure is not null || IsPaused || State?.Error is not null;

    /// <summary>Whether the step shows on <paramref name="host"/> now.</summary>
    public bool IsShowingStep(ConsentHost host) => Pending is not null && Host == host;

    /// <summary>The line to show for a failure or a pause, if there is one (null: nothing wrong).</summary>
    public string? Problem
    {
        get
        {
            var what = FeatureName(Feature);
            // "Summaries and Ask" are two things.
            var meetings = Feature == LlmFeature.Meetings;
            var (it, stays, isOff, uses, was) = meetings
                ? ("them", "stay", "are", "use", "they were")
                : ("it", "stays", "is", "uses", "it was");
            switch (Failure)
            {
                case ConsentFailure.Read:
                    return $"Couldn't read your {what} setting. Open Settings again to retry.";
                case ConsentFailure.Write:
                    return $"Couldn't save the change, so {what} {stays} as {was}.";
                case ConsentFailure.Allow:
                    return $"Couldn't turn {what} on, so {(meetings ? "they" : "it")} {stays} off. Try again.";
                default:
                    break;
            }
            if (State?.Error is string error)
            {
                // An unread switch counts as off, an unread consent as none.
                return $"{Sentence(error)}, so {what} {isOff} off or paused. Turn {it} on again to allow {it}.";
            }
            if (IsPaused && Destination is ConsentDestination destination)
            {
                var words = meetings ? "meeting transcripts" : "your words";
                return destination.IsOnDevice
                    ? $"Paused: {what} now {uses} {destination.Label}. Turn {it} on again to allow {it}."
                    : $"Paused: {what} would now send {words} to {destination.Label}. Turn {it} on again to allow {it}.";
            }
            return null;
        }
    }

    // The consent step's words.

    public static string FeatureName(LlmFeature feature) => feature switch
    {
        LlmFeature.Polish => "polish",
        LlmFeature.Edit => "voice edit",
        _ => "summaries and Ask",
    };

    public static string Title(LlmFeature feature) => feature switch
    {
        LlmFeature.Polish => "Turn on polish?",
        LlmFeature.Edit => "Turn on voice edit?",
        _ => "Turn on summaries and Ask?",
    };

    /// <summary>What Narrator hears as an inline step appears: its heading, then what it says (where the words go).</summary>
    public static string Announcement(LlmFeature feature, ConsentDestination destination) =>
        $"{Title(feature)} {Message(feature, destination)}";

    /// <summary>
    /// Focus and Enter land on Cancel for a model off this PC, so Enter never agrees to send words
    /// away and agreeing is a deliberate press; on the agreeing button for one on this PC.
    /// </summary>
    public static bool FocusesCancel(ConsentDestination destination)
    {
        ArgumentNullException.ThrowIfNull(destination);
        return !destination.IsOnDevice;
    }

    /// <summary>What the consent step says: what the feature sends, and where the words go for this model.</summary>
    public static string Message(LlmFeature feature, ConsentDestination destination)
    {
        ArgumentNullException.ThrowIfNull(destination);
        var what = feature switch
        {
            LlmFeature.Polish => "Polish sends what you dictate to a language model, which tidies the wording before it is typed.",
            LlmFeature.Edit => "Voice edit sends the text you select and what you say to a language model, which rewrites the selection.",
            _ => "Summaries and Ask send a meeting's transcript, what everyone in it said, to a language model, which writes the summary and what was promised, and answers your questions.",
        };
        // A meeting's transcript holds everyone's words, not only the user's.
        var (words, stay, leave) = feature == LlmFeature.Meetings
            ? ("the transcript", "stays", "leaves this PC and goes")
            : ("your words", "stay", "leave this PC and go");
        return destination.IsOnDevice
            ? $"{what} It uses {destination.Label}, so {words} {stay} on this PC."
            : $"{what} It uses {destination.Label}, a cloud provider: {words} {leave} to {destination.Label}.";
    }

    /// <summary>The button that agrees.</summary>
    public static string Button(LlmFeature feature, ConsentDestination destination)
    {
        ArgumentNullException.ThrowIfNull(destination);
        if (!destination.IsOnDevice)
        {
            return $"Send to {destination.Label}";
        }
        return feature switch
        {
            LlmFeature.Polish => "Turn On Polish",
            LlmFeature.Edit => "Turn On Voice Edit",
            _ => "Turn On Summaries and Ask",
        };
    }

    /// <summary>The Cancel button's accessible name.</summary>
    public static string CancelName(LlmFeature feature) => $"Cancel, and leave {FeatureName(feature)} off";

    /// <summary>The agreeing button's accessible name.</summary>
    public static string AllowName(LlmFeature feature, ConsentDestination destination)
    {
        ArgumentNullException.ThrowIfNull(destination);
        var what = FeatureName(feature);
        // Meetings send the whole transcript (everyone's words), as the message says.
        return destination.IsOnDevice
            ? $"Turn on {what} with {destination.Label}"
            : $"Turn on {what} and send {(feature == LlmFeature.Meetings ? "the transcript" : "your words")} to {destination.Label}";
    }

    // Commands.

    /// <summary>A new ref for a request of <paramref name="kind"/> ("get" or "allow"), now the newest.</summary>
    private string NextRef(string kind)
    {
        requests++;
        var reference = $"consent.{kind}:{Wire.Name(Feature)}:{requests}";
        newestRef = reference;
        return reference;
    }

    /// <summary>Reads the state from the core.</summary>
    public void Load()
    {
        var reference = NextRef("get");
        newestGet = reference;
        send(new CoreCommand.ConsentGet(Feature, reference));
    }

    /// <summary>
    /// Shows the consent step for the model now (nothing is sent until Allow). <paramref name="key"/>
    /// is voice edit's. Does nothing while no model is named.
    /// </summary>
    public void Ask(ConsentHost host, string? key = null)
    {
        if (Destination is not ConsentDestination destination)
        {
            return;
        }
        ClearStep();
        Failure = null;
        // An agreement waiting for its choice is not this step's: it is dropped, so it can never
        // allow anything after this.
        Agreed = null;
        Pending = destination;
        Host = host;
        PendingKey = key;
        Asked++;
        Changed();
    }

    /// <summary>
    /// Shows the consent step for <paramref name="destination"/>, the model <paramref name="choose"/>
    /// will choose (the first run's Use Groq): nothing is sent until Allow, which runs
    /// <paramref name="choose"/> and sends the consent once the core names that destination.
    /// <paramref name="choose"/> says whether it sent the choice (false: what it would choose is no
    /// longer what the step named).
    /// </summary>
    public void Ask(ConsentDestination destination, ConsentHost host, Func<bool> choose)
    {
        ArgumentNullException.ThrowIfNull(destination);
        ArgumentNullException.ThrowIfNull(choose);
        Failure = null;
        Agreed = null;
        Pending = destination;
        Host = host;
        PendingKey = null;
        Choosing = true;
        this.choose = choose;
        Asked++;
        Changed();
    }

    /// <summary>
    /// The consent step's Allow: the user agreed to where the feature sends. The core records it
    /// only if that is still where the model goes. A step naming a model not chosen yet chooses it
    /// first, and the consent waits for the core to name it.
    /// </summary>
    public void Allow()
    {
        if (Pending is not ConsentDestination destination)
        {
            return;
        }
        var key = PendingKey;
        var chooseFirst = Choosing ? choose : null;
        ClearStep();
        Failure = null;
        if (chooseFirst is not null)
        {
            // Waiting only for a choice that went: one that did not would leave an agreement that
            // a later state could act on, with no step on screen.
            if (chooseFirst())
            {
                Agreed = destination;
            }
            else
            {
                Failure = ConsentFailure.Allow;
            }
            Changed();
            return;
        }
        SendAllow(destination, key);
        Changed();
    }

    private void SendAllow(ConsentDestination destination, string? key)
    {
        var reference = NextRef("allow");
        newestAllow = reference;
        send(new CoreCommand.ConsentAllow(Feature, destination.Kind, destination.IsOnDevice ? null : destination.Endpoint, key, reference));
    }

    /// <summary>The consent step's Cancel (or the step dismissed): nothing is sent.</summary>
    public void Cancel()
    {
        ClearStep();
        Changed();
    }

    private void ClearStep()
    {
        Pending = null;
        Host = null;
        PendingKey = null;
        Choosing = false;
        choose = null;
    }

    /// <summary>
    /// The owner sent the command that turns the switch off (which withdraws the consent too):
    /// shown at once, put back if the core refuses it.
    /// </summary>
    public void SwitchedOff()
    {
        ClearStep();
        // Off withdraws the consent: an agreement still waiting for its choice goes with it.
        Agreed = null;
        Failure = null;
        if (!hasBeforeOff)
        {
            beforeOff = State;
            hasBeforeOff = true;
        }
        if (State is not null)
        {
            State = State with { On = false, Allowed = false };
        }
        Changed();
    }

    /// <summary>Whether this model shows <paramref name="failed"/> (its reads, its allows and its switch-off).</summary>
    public bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        var feature = Wire.Name(Feature);
        return failed.Command switch
        {
            "consent.get" => failed.Id?.StartsWith($"consent.get:{feature}:", StringComparison.Ordinal) ?? false,
            "consent.allow" => failed.Id?.StartsWith($"consent.allow:{feature}:", StringComparison.Ordinal) ?? false,
            "setting.set" => failed.Id == SwitchSettingId,
            _ => false,
        };
    }

    public void Apply(InkEvent e)
    {
        if (Fold(e))
        {
            Changed();
        }
    }

    private bool Fold(InkEvent e)
    {
        switch (e)
        {
            case EngineRegistered { Kind: EngineKind.Llm }:
            case EngineUnregistered:
                // Where the feature sends may have changed.
                Load();
                return false;
            case CoreStopped:
                ClearStep();
                Agreed = null;
                return true;
            case ConsentState value when value.Feature == Feature:
                // The answer to an older request, arriving after a newer one was sent: the newer
                // answer is the one to show. A state with no ref (after a switch-off, or a refused
                // allow) is the core's own, sent in order, and always applies.
                if (value.Ref is not null && value.Ref != newestRef)
                {
                    return false;
                }
                State = ConsentSnapshot.From(value);
                beforeOff = null;
                hasBeforeOff = false;
                if (Failure is ConsentFailure.Read or ConsentFailure.Write)
                {
                    Failure = null;
                }
                // The step names a destination that is no longer the model's: close it rather than
                // change its words under the user's finger. Turning the feature on asks about the new one.
                // A step that chooses its model names one the core does not know yet: it stays.
                if (Pending is not null && !Choosing && Pending != Destination)
                {
                    ClearStep();
                }
                // The model the user allowed is chosen: the consent goes, for the destination the
                // core names, only if it is the one agreed to (the core then records it only if it
                // still is). The core's own state after the choice (no ref) naming another settles it
                // as refused; an answer to an older read is not about the choice.
                if (Agreed is ConsentDestination agreed)
                {
                    if (Destination is ConsentDestination now && now.Kind == agreed.Kind && now.Endpoint == agreed.Endpoint)
                    {
                        Agreed = null;
                        SendAllow(now, null);
                    }
                    else if (value.Ref is null)
                    {
                        Agreed = null;
                        Failure = ConsentFailure.Allow;
                    }
                }
                return true;
            case CommandFailed failed when failed.Id == SwitchSettingId && failed.Command == "setting.set":
                if (hasBeforeOff)
                {
                    Failure = ConsentFailure.Write;
                    State = beforeOff;
                }
                beforeOff = null;
                hasBeforeOff = false;
                return true;
            case CommandFailed failed when failed.Id is not null && failed.Id == newestGet:
                Failure = ConsentFailure.Read;
                return true;
            case CommandFailed failed when failed.Id is not null && failed.Id == newestAllow:
                Failure = ConsentFailure.Allow;
                return true;
            case CommandFailed { Command: "llm.choose" } when Agreed is not null:
                // The choice the agreement waits for was refused (CloudModel says why): it is
                // dropped, or choosing that model later, where nothing asks, would allow it.
                Agreed = null;
                return true;
            case DictationWarningEvent { Kind: DictationWarning.PolishNotAllowed } when Feature == LlmFeature.Polish:
            case DictationEditFailed { Reason: EditFailure.NotAllowed } when Feature == LlmFeature.Edit:
            case MeetingWarningEvent { Kind: MeetingWarning.SummaryNotAllowed } when Feature == LlmFeature.Meetings:
                // The core refused to send: read where the feature goes now, so the screen says why.
                Load();
                return false;
            default:
                return false;
        }
    }

    /// <summary>"couldn't read x" as the start of a sentence.</summary>
    private static string Sentence(string s) => s.Length == 0 ? s : char.ToUpperInvariant(s[0]) + s[1..];
}
