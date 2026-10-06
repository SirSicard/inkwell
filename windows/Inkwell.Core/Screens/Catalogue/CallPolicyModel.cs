// Per-app call recording: what happens when an app opens the microphone for a call. As the Mac's
// CallPolicyModel. Each app is Always (recorded at once, visibly), Ask (the consent Drop offers it)
// or Never, or follows the default for apps not chosen for. The user sets an app from the Drop
// ("Always for Zoom", "Never for Zoom") or in Settings > Meetings, which also holds the default and
// the list of apps the core has seen.
//
// The core keeps the list (meetings.calls) and answers every change with all of it; this model
// shows the last answer, and a choice in flight until its answer comes. An app is never shown by
// its identity (its executable's name): by the name detection saw, else as Modes names one.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>What the user can choose for one app: a policy of its own, or the default.</summary>
public enum CallChoice
{
    Default,
    Always,
    Ask,
    Never,
}

/// <summary>Where a call-policy change was asked for, so its failure shows there and nowhere else.</summary>
public enum CallPolicyOrigin
{
    Drop,
    Settings,
}

/// <summary>One app's row in Settings > Meetings.</summary>
/// <param name="Id">Its identity, as detection reports it. Never shown.</param>
/// <param name="Label">Its name and icon.</param>
/// <param name="Choice">What the user chose (or Default), a choice in flight shown as made.</param>
/// <param name="Seen">When detection last saw it hold the microphone for a call.</param>
public sealed record CallAppRow(string Id, AppLabel Label, CallChoice Choice, DateTimeOffset? Seen);

public static class CallPolicies
{
    /// <summary>The choices, in the pickers' order.</summary>
    public static IReadOnlyList<CallChoice> Choices { get; } = [CallChoice.Default, CallChoice.Always, CallChoice.Ask, CallChoice.Never];

    /// <summary>The policies, in the default's picker order.</summary>
    public static IReadOnlyList<CallPolicy> All { get; } = [CallPolicy.Always, CallPolicy.Ask, CallPolicy.Never];

    public static string Title(this CallPolicy policy) => policy switch
    {
        CallPolicy.Always => "Always",
        CallPolicy.Ask => "Ask",
        CallPolicy.Never => "Never",
        _ => throw new ArgumentOutOfRangeException(nameof(policy)),
    };

    /// <summary>Its value in the core's commands.</summary>
    public static string Value(this CallChoice choice) => choice switch
    {
        CallChoice.Default => "default",
        CallChoice.Always => "always",
        CallChoice.Ask => "ask",
        CallChoice.Never => "never",
        _ => throw new ArgumentOutOfRangeException(nameof(choice)),
    };

    public static string Value(this CallPolicy policy) => policy switch
    {
        CallPolicy.Always => "always",
        CallPolicy.Ask => "ask",
        CallPolicy.Never => "never",
        _ => throw new ArgumentOutOfRangeException(nameof(policy)),
    };

    public static CallChoice Choice(this CallPolicy policy) => policy switch
    {
        CallPolicy.Always => CallChoice.Always,
        CallPolicy.Ask => CallChoice.Ask,
        _ => CallChoice.Never,
    };

    /// <summary>The policy a choice names; null for Default.</summary>
    public static CallPolicy? Policy(this CallChoice choice) => choice switch
    {
        CallChoice.Always => CallPolicy.Always,
        CallChoice.Ask => CallPolicy.Ask,
        CallChoice.Never => CallPolicy.Never,
        _ => null,
    };

    /// <summary>The policy a stored value names; null for anything else.</summary>
    public static CallPolicy? Parse(string? value) => value switch
    {
        "always" => CallPolicy.Always,
        "ask" => CallPolicy.Ask,
        "never" => CallPolicy.Never,
        _ => null,
    };
}

public sealed class CallPolicyModel(Action<CoreCommand> send, IAppDirectory? apps = null) : ObservableModel
{
    /// <summary>What the core calls an app whose name and identity show nothing.</summary>
    public const string Nameless = "an app";

    public const string DefaultTitle = "Calls in other apps";
    public const string DefaultCaption = "For every app you haven't chosen for below. Ask offers to record each call; Always records it at once, and shows it is recording; Never does neither.";
    /// <summary>Under a default of Always: it records without asking, so the others must be told.</summary>
    public const string AlwaysWarning = "Inkwell records these calls as soon as they start, without asking. Tell the people on the call that you are recording. When Inkwell can't hear an app alone, it asks instead.";
    /// <summary>Under a default of Never.</summary>
    public const string NeverHint = "Inkwell won't offer to record calls in these apps. Record now, on Today, still records.";
    public const string AppsTitle = "Apps";
    public const string NoApps = "None yet. An app shows here once it opens the microphone for a call.";
    public const string UnreadableFromDrop = "Inkwell couldn't read what you chose for each app. Choose in Settings > Meetings.";
    public const string StartOverTitle = "Start the list over?";
    public const string StartOverDetail = "Inkwell couldn't read what you chose for each app. Saving this choice starts the list over: every other app follows the default until you choose again.";
    public const string StartOverButton = "Start over and save";

    /// <summary>The id of the default's setting commands (CoreCommand gives each setting command one).</summary>
    public static string DefaultSettingId => ShellSetting.MeetingsCallsDefault.CommandId();

    private readonly IAppDirectory directory = apps ?? NoInstalledApps.Instance;
    /// <summary>Each set in flight: its app, what was chosen, where, and what follows once it is saved.</summary>
    private readonly Dictionary<string, InFlight> inFlight = new(StringComparer.Ordinal);
    /// <summary>Choices in flight, by app: shown as made until the core answers.</summary>
    private readonly Dictionary<string, CallChoice> pending = new(StringComparer.Ordinal);
    private int nextRef;
    /// <summary>The set that starts an unreadable list over, until its answer.</summary>
    private string? startOverRef;
    /// <summary>
    /// The app the core offers now (meeting.detected, until it is withdrawn or a meeting starts):
    /// what follows an Always saved from the Drop runs only while its offer still stands.
    /// </summary>
    private string? offered;

    /// <summary>The policy for apps not chosen for; null until the core says.</summary>
    public CallPolicy? Default { get; private set; }

    /// <summary>The apps, most recently seen first, as the core last listed them.</summary>
    public IReadOnlyList<CallApp> Apps { get; private set; } = [];

    /// <summary>Whether the core has listed them.</summary>
    public bool Loaded { get; private set; }

    /// <summary>Why the stored choices could not be read (the core's words): every app follows the default meanwhile, and a choice starts the list over after the user confirms it.</summary>
    public string? Unreadable { get; private set; }

    /// <summary>What the core said about the default when it started the list over (it lowered Always to Ask).</summary>
    public string? Note { get; private set; }

    /// <summary>The last failure asked for in Settings.</summary>
    public string? Failure { get; private set; }

    /// <summary>The last failure asked for from the Drop.</summary>
    public string? DropFailure { get; private set; }

    /// <summary>A choice that would start an unreadable list over, waiting for the user to confirm it.</summary>
    public (string App, CallChoice Choice)? StartingOver { get; private set; }

    private string NextRef() => $"calls:{++nextRef}";

    public void Load() => send(new CoreCommand.MeetingsCallsList(NextRef()));

    /// <summary>The default for apps not chosen for.</summary>
    public void SetDefault(CallPolicy policy)
    {
        Failure = null;
        Note = null;
        Default = policy;
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.MeetingsCallsDefault, policy.Value()));
    }

    /// <summary>What happens for <paramref name="app"/> now, when the core has listed it.</summary>
    public CallPolicy? PolicyOf(string app)
    {
        ArgumentNullException.ThrowIfNull(app);
        if (pending.TryGetValue(app, out var choice))
        {
            return choice == CallChoice.Default ? Default : choice.Policy();
        }
        return Apps.FirstOrDefault(a => a.App == app)?.Policy;
    }

    /// <summary>
    /// Chooses for one app; <paramref name="saved"/> runs once the core has saved it (never when it
    /// failed). Over a list the core cannot read, it waits for <see cref="ConfirmStartOver"/>.
    /// </summary>
    public void Choose(CallChoice choice, string app, CallPolicyOrigin origin, Action? saved = null)
    {
        ArgumentNullException.ThrowIfNull(app);
        ClearFailure(origin);
        if (Unreadable is not null)
        {
            // Starting the list over asks first, in Settings; the Drop has no room to ask.
            if (origin == CallPolicyOrigin.Settings)
            {
                StartingOver = (app, choice);
            }
            else
            {
                DropFailure = UnreadableFromDrop;
            }
            Changed();
            return;
        }
        Set(choice, app, origin, replaceUnreadable: false, saved);
    }

    /// <summary>The user agreed to start the unreadable list over with <paramref name="shown"/>, the choice the dialog showed.</summary>
    public void ConfirmStartOver((string App, CallChoice Choice) shown)
    {
        StartingOver = null;
        Set(shown.Choice, shown.App, CallPolicyOrigin.Settings, replaceUnreadable: true, saved: null);
    }

    public void CancelStartOver()
    {
        if (StartingOver is null)
        {
            return;
        }
        StartingOver = null;
        Changed();
    }

    private void Set(CallChoice choice, string app, CallPolicyOrigin origin, bool replaceUnreadable, Action? saved)
    {
        var id = NextRef();
        if (replaceUnreadable)
        {
            startOverRef = id;
        }
        inFlight[id] = new InFlight(app, choice, origin, saved);
        pending[app] = choice;
        Changed();
        send(new CoreCommand.MeetingsCallsSet(app, choice.Value(), replaceUnreadable, id));
    }

    private void ClearFailure(CallPolicyOrigin origin)
    {
        if (origin == CallPolicyOrigin.Drop)
        {
            DropFailure = null;
        }
        else
        {
            Failure = null;
        }
    }

    /// <summary>The rows Settings > Meetings lists.</summary>
    public IReadOnlyList<CallAppRow> Rows => Apps.Select(app =>
    {
        var label = AppIdentity.Label(app.App, directory);
        // An executable is named as Modes names it (installed, well known, else its stem); another
        // identity (a packaged app's id) by the name detection saw, unless that is the core's
        // stand-in for an app with none.
        if (!label.Installed && !AppIdentity.IsExe(app.App) && app.AppName is { Length: > 0 } seen && seen != Nameless)
        {
            label = label with { Name = seen };
        }
        var choice = pending.TryGetValue(app.App, out var made) ? made : app.Chosen ? app.Policy.Choice() : CallChoice.Default;
        return new CallAppRow(app.App, label, choice, app.SeenUnixMs is long ms ? DateTimeOffset.FromUnixTimeMilliseconds(ms) : null);
    }).ToList();

    /// <summary>What a choice reads as in an app's picker: the default names what it is now.</summary>
    public string Title(CallChoice choice) => choice switch
    {
        CallChoice.Default => $"Default ({(Default ?? CallPolicy.Ask).Title()})",
        _ => choice.Policy()!.Value.Title(),
    };

    /// <summary>While the stored choices cannot be read (<paramref name="why"/>, the core's words).</summary>
    public static string UnreadableLine(string why) =>
        $"Inkwell couldn't read what you chose for each app, so every app follows the default for now (Always asks first). Choosing for an app starts the list over. ({why})";

    /// <summary>An app's caption: when it last opened the microphone for a call.</summary>
    public static string? SeenCaption(DateTimeOffset? seen, DateTimeOffset now, IFormatProvider culture)
    {
        if (seen is not DateTimeOffset at)
        {
            return null;
        }
        var local = at.ToLocalTime();
        var today = now.ToLocalTime();
        if (local.Date == today.Date)
        {
            return "Last call today";
        }
        var format = local.Year == today.Year ? "d MMM" : "d MMM yyyy";
        return $"Last call {local.ToString(format, culture)}";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case MeetingsCalls calls:
                Default = calls.Default;
                Apps = calls.Apps;
                Loaded = true;
                var message = string.IsNullOrEmpty(calls.Message) ? null : calls.Message;
                InFlight? answered = null;
                if (calls.Ref is string answeredRef && inFlight.Remove(answeredRef, out var done))
                {
                    if (pending.TryGetValue(done.App, out var shown) && shown == done.Choice)
                    {
                        pending.Remove(done.App);
                    }
                    answered = done;
                }
                if (calls.Ref is not null && calls.Ref == startOverRef)
                {
                    // The list was started over and is readable again; under a default of Always the
                    // answer says the default is Ask now, so the user sets Always again knowingly.
                    startOverRef = null;
                    Unreadable = null;
                    Note = message;
                }
                else
                {
                    Unreadable = message;
                }
                Changed();
                // After the state is the answer's (an Always saved from the Drop records the call), and
                // only while the call is still offered: one that ended meanwhile is not recorded.
                if (answered?.Saved is { } saved && offered == answered.App)
                {
                    saved();
                }
                break;
            case MeetingDetected detected:
                // A new offer: a failure said for the last one is over.
                offered = detected.App;
                ClearDropFailure();
                break;
            case MeetingDetectionEnded ended:
                if (offered == ended.App)
                {
                    offered = null;
                }
                ClearDropFailure();
                break;
            case MeetingDetection { Listening: false }:
                offered = null;
                break;
            case MeetingStarted:
                // The offer was taken (or a meeting started otherwise): its failure goes with it.
                offered = null;
                ClearDropFailure();
                break;
            case CoreStopped:
                offered = null;
                // A core that starts again answers none of the old one's commands: nothing is in
                // flight, and what is stored is read again at core.ready.
                inFlight.Clear();
                pending.Clear();
                startOverRef = null;
                StartingOver = null;
                Changed();
                break;
            case SettingValue value when value.Key == ShellSetting.MeetingsCallsDefault.Key():
                Default = CallPolicies.Parse(value.Value) ?? CallPolicy.Ask;
                Changed();
                break;
            case CommandFailed failed when failed.Command == "meetings.calls.set":
                InFlight? was = null;
                if (failed.Id is string id && inFlight.Remove(id, out var gone))
                {
                    was = gone;
                    if (pending.TryGetValue(gone.App, out var showing) && showing == gone.Choice)
                    {
                        pending.Remove(gone.App);
                    }
                }
                if (failed.Id is not null && failed.Id == startOverRef)
                {
                    startOverRef = null;
                }
                var words = $"Couldn't save that: {failed.Message}";
                if (was?.Origin == CallPolicyOrigin.Drop)
                {
                    DropFailure = words;
                }
                else
                {
                    Failure = words;
                }
                Changed();
                // What is stored now, whatever the failure (a list that became unreadable says so).
                Load();
                break;
            case CommandFailed failed when failed.Command == "meetings.calls.list":
                Failure = $"Couldn't read the apps you chose for: {failed.Message}";
                Changed();
                break;
            case CommandFailed failed when failed.Id == DefaultSettingId:
                Failure = $"Couldn't save the default: {failed.Message}";
                Changed();
                // What it is, not what was asked.
                send(new CoreCommand.SettingGet(ShellSetting.MeetingsCallsDefault));
                break;
            default:
                break;
        }
    }

    private void ClearDropFailure()
    {
        if (DropFailure is not null)
        {
            DropFailure = null;
            Changed();
        }
    }

    private sealed record InFlight(string App, CallChoice Choice, CallPolicyOrigin Origin, Action? Saved);

    /// <summary>Whether this model shows the failure.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "meetings.calls.list" or "meetings.calls.set" || failed.Id == DefaultSettingId;
    }
}
