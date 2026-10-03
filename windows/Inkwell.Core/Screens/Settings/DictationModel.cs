// Dictation as Settings > Voice shows it: whether it is live and on which keys, and the choice of
// keys. A port of the Settings half of the Mac's DictationModel; what the Drop says about a take
// (the Mac's DictationModel.note and its per-take texts) belongs to the Drop and is not here.
//
// The core holds the keys and the mic (architecture rule 1). Once the core is ready the shell reads
// the user's switch (dictation.enabled, never set: on) and, unless it is off, sends
// dictation.enable; again when the app becomes active while dictation is off for a reason turning
// it on again may fix. Turning the switch off sends dictation.disable, which lets go of the keys
// and the mic (quitting needs nothing: the core lets go of them when it stops). The keys go through
// setting.set, and the core rebinds at once and answers dictation.ready or dictation.off. Nothing
// here polls.
//
// Windows: the quick picks are the right-hand modifiers held on their own (right_control, the
// default, right_alt, right_shift, right_win); any other key the core accepts can be recorded
// (ShortcutRecorderModel), and shows in the pickers as KeyNotation writes it. A lost key is the
// low-level keyboard hook that Windows removed and the core could not put back, not a permission,
// so it says so and turning dictation on again (or coming back to the app) retries. While a
// shortcut is recorded dictation is paused (SuspendForRecording), so the key can't start a take.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A key a user can pick.</summary>
/// <param name="Token">The core's token.</param>
/// <param name="Name">Its name, spelled out.</param>
/// <param name="Cap">How the key cap reads.</param>
public sealed record DictationKey(string Token, string Name, string Cap);

/// <summary>Where dictation is.</summary>
public abstract record DictationState
{
    private DictationState() { }

    /// <summary>Not answered yet.</summary>
    public sealed record Starting : DictationState;

    /// <summary>Live: the core holds these keys.</summary>
    public sealed record Live(string Key, string? EditKey) : DictationState;

    /// <summary>Not live, and why.</summary>
    public sealed record Off(DictationOffReason Reason, string? Message) : DictationState;
}

/// <summary>A dictation.enable or dictation.disable that failed as a command.</summary>
public enum DictationCommandFailure
{
    Enable,
    Disable,
}

public sealed class DictationModel : ObservableModel
{
    /// <summary>The dictation key on Windows until the user picks another (the core's default).</summary>
    public const string DefaultKey = "right_control";

    /// <summary>The keys a user can pick (the core's tokens: right-hand modifiers held on their own).</summary>
    public static IReadOnlyList<DictationKey> Keys { get; } =
    [
        new("right_control", "Right Ctrl", "Right Ctrl"),
        new("right_alt", "Right Alt", "Right Alt"),
        new("right_shift", "Right Shift", "Right Shift"),
        new("right_win", "Right Windows key", "Right Win"),
    ];

    public const string KeyLostText =
        "The dictation key stopped working: Windows stopped sending it to Inkwell. Turn dictation on again to get it back.";

    public const string EditKeyLostText =
        "The edit key stopped working: Windows stopped sending it to Inkwell. Turn dictation on again to get it back.";

    /// <summary>The prefix of this model's dictation command ids.</summary>
    public const string RefPrefix = "dictation:";

    private readonly Action<CoreCommand> send;
    private readonly Func<TimeSpan> utcOffset;
    private int nextRef;

    /// <param name="utcOffset">The local offset from UTC now (for {date} and {time} in snippets); the system's by default.</param>
    public DictationModel(Action<CoreCommand> send, Func<TimeSpan>? utcOffset = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.utcOffset = utcOffset ?? (() => TimeZoneInfo.Local.GetUtcOffset(DateTimeOffset.UtcNow));
    }

    public static string KeySettingId => ShellSetting.DictationKey.CommandId();

    public static string EditKeySettingId => ShellSetting.DictationEditKey.CommandId();

    public static string EnabledSettingId => ShellSetting.DictationEnabled.CommandId();

    /// <summary>A token's key, for showing it.</summary>
    public static DictationKey? Key(string? token) => Keys.FirstOrDefault(k => k.Token == token);

    /// <summary>How a token reads on a key cap, in Windows notation ("ctrl+shift+space" reads "Ctrl+Shift+Space"; KeyNotation).</summary>
    public static string Cap(string token)
    {
        ArgumentNullException.ThrowIfNull(token);
        return Key(token)?.Cap ?? KeyNotation.Describe(token).Cap;
    }

    /// <summary>A shortcut is being recorded: dictation is paused until it ends.</summary>
    public bool SuspendedForRecording { get; private set; }

    /// <summary>
    /// A shortcut is being recorded: the keys are let go of, or the current key would start a take
    /// and the core's hook would swallow it before the recorder saw it. Nothing turns them on again
    /// until the recording ends (Enable waits), whatever the switch says meanwhile.
    /// </summary>
    public void SuspendForRecording()
    {
        if (SuspendedForRecording)
        {
            return;
        }
        SuspendedForRecording = true;
        if (WantsOn == true)
        {
            Disable();
        }
        Changed();
    }

    /// <summary>
    /// The recording ended (saved, refused or cancelled): dictation comes back, on the key just
    /// saved if one was (setting.set was sent first, and the core runs both in order).
    /// </summary>
    public void ResumeAfterRecording()
    {
        if (!SuspendedForRecording)
        {
            return;
        }
        SuspendedForRecording = false;
        if (WantsOn == true)
        {
            Enable();
        }
        Changed();
    }

    public DictationState State { get; private set; } = new DictationState.Starting();

    /// <summary>The dictation key the user chose (null until read; the core's default is right_control).</summary>
    public string? KeySetting { get; private set; }

    /// <summary>The edit key the user chose, or "off".</summary>
    public string? EditKeySetting { get; private set; }

    /// <summary>Why the chosen edit key is not held.</summary>
    public string? EditKeyProblem { get; private set; }

    /// <summary>Settings dictation could not read (it runs with their defaults).</summary>
    public string? SettingsProblem { get; private set; }

    /// <summary>A key setting could not be read or saved.</summary>
    public string? KeyFailure { get; private set; }

    /// <summary>Windows stopped sending the dictation key: not working until dictation is enabled again.</summary>
    public bool KeyLost { get; private set; }

    /// <summary>The user's switch: null until read, then whether dictation should be live.</summary>
    public bool? WantsOn { get; private set; }

    /// <summary>
    /// A dictation.enable or dictation.disable that failed as a command (never ran, or a bug in the
    /// core stopped it), until the core answers the next one.
    /// </summary>
    public DictationCommandFailure? CommandFailure { get; private set; }

    /// <summary>Turns dictation on (or, when on, rebinds its keys). Waits while a shortcut is recorded.</summary>
    public void Enable()
    {
        if (SuspendedForRecording)
        {
            return;
        }
        nextRef++;
        var minutes = (int)Math.Round(utcOffset().TotalMinutes);
        send(new CoreCommand.DictationEnable(minutes, $"{RefPrefix}{nextRef}"));
    }

    /// <summary>Lets go of the keys and the mic.</summary>
    public void Disable()
    {
        nextRef++;
        send(new CoreCommand.DictationDisable($"{RefPrefix}{nextRef}"));
    }

    /// <summary>Reads the switch (and, unless it is off, then turns dictation on) and the key settings.</summary>
    public void Load()
    {
        send(new CoreCommand.SettingGet(ShellSetting.DictationEnabled));
        send(new CoreCommand.SettingGet(ShellSetting.DictationKey));
        send(new CoreCommand.SettingGet(ShellSetting.DictationEditKey));
    }

    /// <summary>Whether the switch in Settings > Voice reads on.</summary>
    public bool IsOn => WantsOn != false;

    /// <summary>The line beside the switch.</summary>
    public string SwitchCaption => IsOn ? "The keys below are Inkwell's" : "Off: the keys do what they did before";

    /// <summary>The user turned dictation on or off: kept for the next launch, and done now.</summary>
    public void SetOn(bool on)
    {
        WantsOn = on;
        KeyFailure = null;
        send(new CoreCommand.SettingSet(ShellSetting.DictationEnabled, on ? "on" : "off"));
        if (on)
        {
            Enable();
        }
        else
        {
            Disable();
        }
        Changed();
    }

    /// <summary>Tries again what failed: turning dictation off if that failed, else turning it on.</summary>
    public void Retry()
    {
        if (CommandFailure == DictationCommandFailure.Disable)
        {
            Disable();
        }
        else
        {
            Enable();
        }
    }

    /// <summary>Back in the app: try again what turning dictation on may fix.</summary>
    public void AppBecameActive()
    {
        if (WantsOn == false || SuspendedForRecording)
        {
            return;
        }
        if (State is DictationState.Off { Reason: DictationOffReason.NeedsAccessibility or DictationOffReason.KeyRefused })
        {
            Enable();
        }
        else if (KeyLost || EditKeyProblem == EditKeyLostText)
        {
            Enable();
        }
    }

    public void SetKey(string token)
    {
        KeyFailure = null;
        send(new CoreCommand.SettingSet(ShellSetting.DictationKey, token));
        // Off because the key before was refused: the core binds nothing until asked, so a new
        // key turns dictation on again (after the save, which the core runs first).
        if (State is DictationState.Off { Reason: DictationOffReason.KeyRefused } && WantsOn == true)
        {
            Enable();
        }
        Changed();
    }

    /// <summary><paramref name="token"/> null turns the edit key off.</summary>
    public void SetEditKey(string? token)
    {
        KeyFailure = null;
        send(new CoreCommand.SettingSet(ShellSetting.DictationEditKey, token ?? "off"));
        Changed();
    }

    /// <summary>The key picked now: the one held, else the one chosen, else the core's default.</summary>
    public string CurrentKey => State is DictationState.Live live ? live.Key : KeySetting ?? DefaultKey;

    /// <summary>
    /// The edit key chosen (null: off). Never the dictation key, as the core refuses it: so the edit
    /// picker's selection is always one of its own options (it offers every key but that one).
    /// </summary>
    public string? EditKey
    {
        get
        {
            if (State is DictationState.Live { EditKey: string edit })
            {
                return edit == CurrentKey ? null : edit;
            }
            if (EditKeySetting is not string chosen || chosen == "off" || chosen == CurrentKey)
            {
                return null;
            }
            return chosen;
        }
    }

    /// <summary>The edit picker's keys: every key but the dictation key (Off is the picker's own first item).</summary>
    public IReadOnlyList<DictationKey> EditKeys => Keys.Where(k => k.Token != CurrentKey).ToList();

    /// <summary>
    /// Whether dictation is off, or its keys lost, for a reason turning it on again may fix (the
    /// Voice section offers that). Not for an unsupported build, nor for the user's own switch.
    /// </summary>
    public bool CanRetry
    {
        get
        {
            if (SuspendedForRecording)
            {
                return false;
            }
            if (CommandFailure is not null)
            {
                return true;
            }
            if (State is DictationState.Off off)
            {
                // Off by the user's switch is the switch's to change.
                return off.Reason is DictationOffReason.WorkerStopped or DictationOffReason.Failed
                    or DictationOffReason.Other or DictationOffReason.KeyRefused;
            }
            // Windows: a lost hook comes back when dictation is enabled again.
            return KeyLost || EditKeyProblem == EditKeyLostText;
        }
    }

    /// <summary>The retry button's words.</summary>
    public string RetryTitle => CommandFailure == DictationCommandFailure.Disable ? "Try again" : "Turn dictation on";

    /// <summary>The line under the keys in Settings: a key setting's failure, else the status.</summary>
    public string StatusLine => KeyFailure ?? Status;

    /// <summary>Dictation's state in words.</summary>
    public string Status
    {
        get
        {
            // The user's latest action first: a turn-off that failed outranks an older lost key.
            if (CommandFailure == DictationCommandFailure.Disable)
            {
                return "Dictation couldn't be turned off.";
            }
            if (SuspendedForRecording)
            {
                return "Dictation is paused while you record a shortcut.";
            }
            if (KeyLost)
            {
                return KeyLostText;
            }
            switch (State)
            {
                case DictationState.Live live:
                    return $"Hold {Cap(live.Key)}, speak, let go.";
                case DictationState.Off off:
                    var detail = off.Message is null ? "." : $": {off.Message}";
                    return off.Reason switch
                    {
                        DictationOffReason.NeedsAccessibility => "Dictation needs “Type for you” to hold its key.",
                        DictationOffReason.KeyRefused => $"That key can't be used here{(off.Message is null ? "" : $": {off.Message}")}. Pick another, or record a shortcut.",
                        DictationOffReason.Unsupported => "Dictation isn't available in this build.",
                        DictationOffReason.WorkerStopped => "Dictation stopped after repeated failures.",
                        DictationOffReason.Disabled => "Dictation is off.",
                        _ => $"Dictation couldn't start{detail}",
                    };
                default:
                    return "Starting…";
            }
        }
    }

    /// <summary>The edit key's problem as a line, if it has one.</summary>
    public string? EditKeyProblemLine =>
        EditKeyProblem is null ? null : EditKeyProblem == EditKeyLostText ? EditKeyProblem : $"The edit key isn't held: {EditKeyProblem}";

    /// <summary>Settings the core could not read, as a line.</summary>
    public string? SettingsProblemLine => SettingsProblem is null ? null : $"Dictation {SettingsProblem}, so it uses the defaults for them.";

    /// <summary>Whether the status is a problem to show in the alert colour.</summary>
    public bool IsProblem
    {
        get
        {
            if (State is DictationState.Off off)
            {
                return off.Reason != DictationOffReason.Disabled;
            }
            return KeyFailure is not null || KeyLost || EditKeyProblem is not null || CommandFailure is not null;
        }
    }

    /// <summary>Whether this model shows a failed command itself.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Id == KeySettingId || failed.Id == EditKeySettingId || failed.Id == EnabledSettingId
            || (failed.Id?.StartsWith(RefPrefix, StringComparison.Ordinal) ?? false);
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
            case CoreStopped:
                State = new DictationState.Starting();
                WantsOn = null;
                // The restarted core turns dictation on from its switch; a recording it cut short
                // leaves nothing to resume.
                SuspendedForRecording = false;
                return true;
            case DictationReady ready:
                State = new DictationState.Live(ready.Key, ready.EditKey);
                KeyLost = false;
                CommandFailure = null;
                EditKeyProblem = ready.EditKeyError;
                SettingsProblem = ready.SettingsError;
                return true;
            case DictationOff off:
                State = new DictationState.Off(off.Reason, off.Message);
                KeyLost = false;
                CommandFailure = null;
                return true;
            case SettingValue value when value.Key == ShellSetting.DictationEnabled.Key():
                var first = WantsOn is null;
                WantsOn = value.Value != "off";
                if (first)
                {
                    if (WantsOn == true)
                    {
                        Enable();
                    }
                    else
                    {
                        State = new DictationState.Off(DictationOffReason.Disabled, null);
                    }
                }
                return true;
            case CommandFailed failed when failed.Id == EnabledSettingId:
                if (failed.Command == "setting.get")
                {
                    KeyFailure = "Couldn't read whether dictation is on, so it is on.";
                    if (WantsOn is null)
                    {
                        WantsOn = true;
                        Enable();
                    }
                }
                else
                {
                    KeyFailure = "Couldn't save the switch, so the next launch keeps the one before.";
                }
                return true;
            case CommandFailed failed when failed.Id?.StartsWith(RefPrefix, StringComparison.Ordinal) ?? false:
                if (failed.Command == "dictation.disable")
                {
                    CommandFailure = DictationCommandFailure.Disable;
                }
                else
                {
                    CommandFailure = DictationCommandFailure.Enable;
                    State = new DictationState.Off(DictationOffReason.Failed, null);
                }
                return true;
            // The Drop says these once too (not here); Settings shows them until dictation is enabled again.
            case DictationHotkeyLost:
                KeyLost = true;
                return true;
            case DictationEditHotkeyLost:
                EditKeyProblem = EditKeyLostText;
                return true;
            case SettingValue value when value.Key == ShellSetting.DictationKey.Key():
                KeySetting = value.Value;
                return true;
            case SettingValue value when value.Key == ShellSetting.DictationEditKey.Key():
                EditKeySetting = value.Value;
                return true;
            case CommandFailed failed when failed.Id == KeySettingId || failed.Id == EditKeySettingId:
                KeyFailure = failed.Command == "setting.set"
                    ? "Couldn't save the key. The one before still works."
                    : "Couldn't read your key settings.";
                return true;
            default:
                return false;
        }
    }
}
