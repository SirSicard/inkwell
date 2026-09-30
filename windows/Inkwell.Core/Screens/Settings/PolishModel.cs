// The Polish toggle (Settings > AI, and the first-run sheet): the user's switch, the consent that
// turns it on (ConsentModel, feature polish), and whether a language model can do it. A port of the
// Mac's PolishModel.
//
// Polish sends what the user dictates to a language model before it is typed, so it is off until
// the user turns it on through a consent step that says where the words go (ConsentModel has the
// flow). Switching the toggle on only asks (PendingConsent); Allow sends consent.allow, Cancel
// sends nothing. Off sends setting.set dictation.polish off, which withdraws the consent in the
// same write.
//
// The toggle reads "on" only when the switch is on, the consent covers the model, and the core has
// a working language model: one registered (engine.registered, kind llm, not let go of since), or
// an own-key provider chosen and ready (llm.providers, Settings > AI's language model). With no
// working model it reads off, cannot be switched, and says so. Windows has no Apple Intelligence:
// which models exist is the core's to say (its engines), so the Mac's Apple-engine reasons are gone.
//
// It also tells "polish keeps timing out" from an ordinary failure: a take whose polish ran out of
// its time budget arrives as the dictation warning polish_timed_out, while a polish cancelled for
// another reason (the core shutting down) stays polish_failed.
//
// A state that could not be read, a consent that could not be recorded and a switch that could not
// be saved each say so under the toggle ("couldn't ..."), never read as off.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class PolishModel : ObservableModel
{
    /// <summary>Timeouts in a row at which the toggle warns.</summary>
    public const int TimeoutWarning = 2;

    /// <summary>The line while no language model is registered.</summary>
    public const string NoModelText = "No language model is available on this PC, so polish stays off.";

    private readonly Action<CoreCommand> send;
    /// <summary>Language models the core confirmed and still holds, by id.</summary>
    private readonly HashSet<string> models = new(StringComparer.Ordinal);
    /// <summary>An own-key provider is chosen and can be called (llm.providers' ready).</summary>
    private bool cloudReady;
    private bool takeTimedOut;

    public PolishModel(Action<CoreCommand> send)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        Consent = new ConsentModel(LlmFeature.Polish, SettingId, send);
        Consent.PropertyChanged += (_, _) => Changed();
    }

    /// <summary>The id of this model's switch command (a command.failed carries it).</summary>
    public static string SettingId => ShellSetting.DictationPolish.CommandId();

    /// <summary>Polish's consent and switch, from the core.</summary>
    public ConsentModel Consent { get; }

    /// <summary>Polish timeouts in a row, across takes; a take that polished resets it.</summary>
    public int TimeoutsInARow { get; private set; }

    /// <summary>Whether a language model can polish now (summaries and Ask use the same test).</summary>
    public bool HasWorkingEngine => models.Count > 0 || cloudReady;

    /// <summary>The core's state (null until read).</summary>
    public ConsentSnapshot? State => Consent.State;

    /// <summary>The user's switch, from the core (null until read).</summary>
    public bool? Preference => Consent.State?.On;

    /// <summary>Where polish would send now, if the core named a model.</summary>
    public ConsentDestination? Destination => Consent.Destination;

    /// <summary>Something could not be read, recorded or saved.</summary>
    public ConsentFailure? Failure => Consent.Failure;

    /// <summary>The consent step on screen.</summary>
    public ConsentDestination? PendingConsent => Consent.Pending;

    /// <summary>Which screen asked for the consent step.</summary>
    public ConsentHost? ConsentHost => Consent.Host;

    /// <summary>What the toggle shows: on only with the switch, a consent that covers the model, and a working engine.</summary>
    public bool IsOn => Consent.IsAllowedOn && HasWorkingEngine;

    /// <summary>On, but the model now sends somewhere the user has not agreed to: nothing is polished.</summary>
    public bool IsPaused => Consent.IsPaused && HasWorkingEngine;

    /// <summary>Whether the toggle can be switched.</summary>
    public bool CanToggle => HasWorkingEngine && Consent.State is not null && Destination is not null;

    /// <summary>Polish has timed out on several takes in a row.</summary>
    public bool KeepsTimingOut => TimeoutsInARow >= TimeoutWarning;

    /// <summary>Whether the status is a problem to show in the alert colour.</summary>
    public bool IsProblem => Consent.IsProblem || KeepsTimingOut;

    /// <summary>The line under the toggle.</summary>
    public string Status
    {
        get
        {
            // A failure, or a part of the state the core could not read, is said whether or not
            // polish is on and whether or not a model works: an unread switch must never read as plain off.
            if ((Consent.Failure is not null || Consent.State?.Error is not null) && Consent.Problem is string failed)
            {
                return failed;
            }
            if (!HasWorkingEngine)
            {
                return NoModelText;
            }
            if (Consent.Problem is string problem)
            {
                return problem;
            }
            if (KeepsTimingOut)
            {
                return "Polish keeps timing out, so your words go in as you said them.";
            }
            if (!IsOn || Destination is not ConsentDestination destination)
            {
                return "Off. Your words go in as you said them.";
            }
            return destination.IsOnDevice
                ? $"On, with {destination.Label}. Your words stay on this PC."
                : $"On. Your words go to {destination.Label} before they are typed.";
        }
    }

    // The consent step's words.

    public static string ConsentTitle => ConsentModel.Title(LlmFeature.Polish);

    public static string ConsentMessage(ConsentDestination destination) => ConsentModel.Message(LlmFeature.Polish, destination);

    public static string ConsentButton(ConsentDestination destination) => ConsentModel.Button(LlmFeature.Polish, destination);

    // Commands.

    /// <summary>Reads the state from the core.</summary>
    public void Load() => Consent.Load();

    /// <summary>
    /// The user switched the toggle. On shows the consent step (nothing is sent until Allow); off
    /// turns polish off, which also withdraws the consent. Does nothing without a working engine.
    /// </summary>
    public void SetOn(bool on, ConsentHost from = Screens.ConsentHost.Settings)
    {
        if (!CanToggle)
        {
            return;
        }
        if (on)
        {
            Consent.Ask(from);
            return;
        }
        Consent.SwitchedOff();
        send(new CoreCommand.SettingSet(ShellSetting.DictationPolish, "off"));
    }

    public void AllowConsent() => Consent.Allow();

    public void CancelConsent() => Consent.Cancel();

    /// <summary>Whether this model shows <paramref name="failed"/>: its setting's read or write, and polish's consent commands.</summary>
    public bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return (failed.Command is "setting.get" or "setting.set" && failed.Id == SettingId) || Consent.Handles(failed);
    }

    public void Apply(InkEvent e)
    {
        if (Fold(e))
        {
            Changed();
        }
        Consent.Apply(e);
    }

    private bool Fold(InkEvent e)
    {
        switch (e)
        {
            case EngineRegistered { Kind: EngineKind.Llm } engine:
                return models.Add(engine.Id);
            case EngineUnregistered engine:
                return models.Remove(engine.Id);
            case LlmProviders providers:
                var was = cloudReady;
                cloudReady = providers.Ready;
                return was != cloudReady;
            case CoreStopped:
                models.Clear();
                cloudReady = false;
                return true;
            case DictationStarted:
                takeTimedOut = false;
                return false;
            case DictationWarningEvent { Kind: DictationWarning.PolishTimedOut }:
                takeTimedOut = true;
                TimeoutsInARow++;
                return true;
            case DictationInserted:
                // A take that went out without timing out, while polish was on, ends the run.
                if (!takeTimedOut && IsOn && TimeoutsInARow != 0)
                {
                    TimeoutsInARow = 0;
                    return true;
                }
                return false;
            default:
                return false;
        }
    }
}
