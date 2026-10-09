// "Record a shortcut…" for the dictation key and the edit key (Settings > Dictation): the Mac's
// ShortcutRecorderModel, Windows' version.
//
// Everyone may use any key they like, so the quick picks are a start, not the list. While the user
// records, the next key press is captured (a modifier alone when it comes up with nothing else
// pressed, a function key, or modifiers and a key), named as the core names it, and sent to the
// core's hotkey.check: the core is the one judge of what it can watch. Only a key it accepts is
// saved, in the spelling it answers; a refusal is shown with its reason, the old key stays, and the
// recorder goes on listening for another key (stopping there left the refusal on screen while
// every later press did nothing, which read as every key being refused). Escape on its own cancels. While recording, dictation is off (dictation.disable), or the current
// key would start a take, and the core's hook would swallow it before the recorder saw it; it comes
// back on (dictation.enable, after the save) when recording ends, however it ends. A check that
// gets no answer in 5 s gives up; a core that stops ends the recording.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A key as the recorder's window reports it: a modifier by its side ("left_control", "right_alt"...), or a virtual key.</summary>
public readonly record struct CapturedKey(string? Modifier, uint Vk)
{
    public static CapturedKey Side(string modifier) => new(modifier, 0);

    public static CapturedKey Of(uint vk) => new(null, vk);

    /// <summary>VK_CAPITAL.</summary>
    public const uint CapsLock = 0x14;

    /// <summary>VK_ESCAPE.</summary>
    public const uint Escape = 0x1B;
}

/// <summary>The capture itself: key events in, a token out. No WinUI, so it is tested with plain values.</summary>
public sealed class ShortcutCapture
{
    /// <summary>One key event in the recorder's window.</summary>
    public abstract record Input
    {
        private Input() { }

        /// <param name="Repeat">An auto-repeat (the key was already down).</param>
        public sealed record KeyDown(CapturedKey Key, bool Repeat = false) : Input;

        public sealed record KeyUp(CapturedKey Key) : Input;
    }

    public abstract record Outcome
    {
        private Outcome() { }

        /// <summary>Still listening.</summary>
        public sealed record Listening : Outcome;

        /// <summary>Escape, with no modifier.</summary>
        public sealed record Cancelled : Outcome;

        /// <summary>A token for the core to judge.</summary>
        public sealed record Captured(string Token) : Outcome;

        /// <summary>A key the core has no name for (the keypad, media keys).</summary>
        public sealed record UnknownKey : Outcome;
    }

    /// <summary>Each side's modifier token and its chord name.</summary>
    private static readonly Dictionary<string, string> ChordName = new(StringComparer.Ordinal)
    {
        ["left_control"] = "ctrl",
        ["right_control"] = "ctrl",
        ["left_alt"] = "alt",
        ["right_alt"] = "alt",
        ["left_shift"] = "shift",
        ["right_shift"] = "shift",
        ["left_win"] = "win",
        ["right_win"] = "win",
    };

    /// <summary>The modifier keys down now, and every one pressed since the last all-up, in order.</summary>
    private readonly HashSet<string> down = new(StringComparer.Ordinal);
    private readonly List<string> pressed = [];

    /// <summary>Keys still down from an attempt the core refused: their repeats and releases are not a new press.</summary>
    private readonly HashSet<CapturedKey> stale;

    /// <param name="stillDown">Keys held from the attempt before this one (a refused Ctrl+V's Ctrl, say), whose release must not read as a modifier pressed alone.</param>
    public ShortcutCapture(IEnumerable<CapturedKey>? stillDown = null)
    {
        stale = [.. stillDown ?? []];
    }

    public Outcome Feed(Input input)
    {
        ArgumentNullException.ThrowIfNull(input);
        // A key still down from the refused attempt: its repeats and its release are that
        // attempt's. Pressed afresh, it is a key like any other.
        if (input is Input.KeyDown { Repeat: false } fresh)
        {
            stale.Remove(fresh.Key);
        }
        switch (input)
        {
            case Input.KeyDown { Repeat: true } staleDown when stale.Contains(staleDown.Key):
                return new Outcome.Listening();
            case Input.KeyUp staleUp when stale.Remove(staleUp.Key):
                return new Outcome.Listening();
            // A modifier that is not down yet is a press even when it says it repeats: on a layout
            // with AltGr, WinUI marks right Alt's press, after the left Ctrl Windows makes, a repeat.
            case Input.KeyDown { Repeat: true, Key.Modifier: var held } when held is null || down.Contains(held):
                return new Outcome.Listening();
            case Input.KeyDown { Key.Modifier: string modifier } when ChordName.ContainsKey(modifier):
                down.Add(modifier);
                if (!pressed.Contains(modifier))
                {
                    pressed.Add(modifier);
                }
                return new Outcome.Listening();
            case Input.KeyDown { Key.Vk: CapturedKey.CapsLock }:
                // A switch, not a key that is held: the core says so.
                return new Outcome.Captured("caps_lock");
            case Input.KeyDown { Key.Vk: CapturedKey.Escape } when down.Count == 0:
                return new Outcome.Cancelled();
            case Input.KeyDown { Key.Vk: var vk }:
                if (KeyNotation.ForVk(vk) is not KeyNotation.Key key)
                {
                    return new Outcome.UnknownKey();
                }
                var names = down.Select(m => ChordName[m]).ToHashSet(StringComparer.Ordinal);
                var parts = KeyNotation.ChordOrder.Where(names.Contains).Append(key.Token);
                return new Outcome.Captured(string.Join("+", parts));
            case Input.KeyUp { Key.Modifier: string modifier } when ChordName.ContainsKey(modifier):
                down.Remove(modifier);
                // AltGr (right Alt on many layouts) arrives as left Ctrl and right Alt together, and
                // WinUI never says right Alt came up: the first of the two to come up ends it.
                if (pressed.Count == 2 && pressed.Contains("left_control") && pressed.Contains("right_alt")
                    && modifier is "left_control" or "right_alt")
                {
                    down.Clear();
                    pressed.Clear();
                    return new Outcome.Captured("right_alt");
                }
                if (down.Count > 0 || pressed.Count == 0)
                {
                    return new Outcome.Listening();
                }
                var all = pressed.ToList();
                pressed.Clear();
                if (all.Count == 1)
                {
                    return new Outcome.Captured(all[0]);
                }
                // Modifiers together with no key: the core refuses them and says why.
                var together = all.Select(m => ChordName[m]).ToHashSet(StringComparer.Ordinal);
                return new Outcome.Captured(string.Join("+", KeyNotation.ChordOrder.Where(together.Contains)));
            default:
                return new Outcome.Listening();
        }
    }
}

/// <summary>Which key a recording is for.</summary>
public enum ShortcutTarget
{
    Dictation,
    Edit,
    Meeting,
}

/// <summary>What shows under a key's row after a recording.</summary>
public sealed record ShortcutMessage(string Text, bool IsProblem);

/// <summary>The recorder for both keys, its inline messages, and what it asks of dictation. UI thread only.</summary>
public sealed class ShortcutRecorderModel : ObservableModel
{
    public const string RefPrefix = "hotkey:";

    public const string RecordTitle = "Record a shortcut…";
    public const string RecordingTitle = "Press the keys… (Esc cancels)";
    public const string CancelTitle = "Cancel";

    private readonly Action<CoreCommand> send;
    private readonly DictationModel dictation;
    private readonly Action<string> saveEditKey;
    private readonly IWakeScheduler wake;
    private readonly MeetingShortcutModel? meeting;
    private string? suspensionRef;
    private readonly HashSet<CapturedKey> heldKeys = [];
    private HotkeyChecked? checkedWhileHeld;
    public ShortcutTarget? Capturing => Recording ?? (heldKeys.Count > 0 ? Checking?.Target : null);
    public ShortcutTarget? Waiting { get; private set; }
    public bool Busy => Recording is not null || Checking is not null || Waiting is not null;
    private readonly Dictionary<ShortcutTarget, ShortcutMessage> messages = [];
    private ShortcutCapture capture = new();
    private int nextRef;
    private string? reference;
    private IDisposable? timeout;

    /// <param name="saveEditKey">Saves a recorded edit key (AiSettings.ChooseEditKey: it asks for consent first when voice edit is not on yet).</param>
    /// <param name="wake">Runs the 5 s check timeout on the UI thread.</param>
    public ShortcutRecorderModel(Action<CoreCommand> send, DictationModel dictation, Action<string> saveEditKey, IWakeScheduler wake, MeetingShortcutModel? meeting = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        ArgumentNullException.ThrowIfNull(dictation);
        ArgumentNullException.ThrowIfNull(saveEditKey);
        ArgumentNullException.ThrowIfNull(wake);
        this.send = send;
        this.dictation = dictation;
        this.saveEditKey = saveEditKey;
        this.wake = wake;
        this.meeting = meeting;
    }

    /// <summary>How long the core has to answer a check before the recorder gives up on it.</summary>
    public TimeSpan CheckTimeout { get; set; } = TimeSpan.FromSeconds(5);

    /// <summary>How a token is shown: the user's keyboard layout labels the typing keys.</summary>
    public Func<string, DictationKey> Describe { get; set; } = token => KeyNotation.Describe(token);

    /// <summary>Says something to Narrator, once, when it happens.</summary>
    public Action<string> Announce { get; set; } = _ => { };

    /// <summary>The key being recorded, if any.</summary>
    public ShortcutTarget? Recording { get; private set; }

    /// <summary>A captured key the core is judging.</summary>
    public (ShortcutTarget Target, string Token)? Checking { get; private set; }

    /// <summary>The latest message for <paramref name="target"/>'s key.</summary>
    public ShortcutMessage? Message(ShortcutTarget target) => messages.GetValueOrDefault(target);

    /// <summary>The button beside <paramref name="target"/>'s picker.</summary>
    public string ButtonTitle(ShortcutTarget target) =>
        Waiting == target ? "Waiting for shortcuts to pause…" : Recording == target ? RecordingTitle : Checking?.Target == target ? CancelTitle : RecordTitle;

    /// <summary>The button's name for Narrator, which says what it is for in each state.</summary>
    public string ButtonName(ShortcutTarget target)
    {
        var what = target == ShortcutTarget.Dictation ? "dictation" : target == ShortcutTarget.Meeting ? "meeting recording" : "editing a selection";
        return Recording == target ? $"Recording a shortcut for {what}"
            : Checking?.Target == target ? $"Cancel checking the shortcut for {what}"
            : $"Record a shortcut for {what}";
    }

    /// <summary>The button's hint for Narrator.</summary>
    public string ButtonHint(ShortcutTarget target) =>
        Recording == target ? "Press the keys you want: a modifier alone, a function key, or modifiers and a key. Escape on its own cancels."
            : Checking?.Target == target ? ""
            : "Then press the keys you want to use.";

    private static string Spoken(ShortcutTarget target) => target == ShortcutTarget.Dictation ? "the dictation key" : target == ShortcutTarget.Meeting ? "the meeting key" : "the edit key";

    /// <summary>Starts recording <paramref name="target"/>'s key, or (pressed again, while recording or checking) stops.</summary>
    public void Toggle(ShortcutTarget target)
    {
        if (Recording == target || Checking?.Target == target || Waiting == target)
        {
            Cancel();
            Announce("Recording cancelled. The key is unchanged.");
        }
        else
        {
            Start(target);
        }
    }

    public void Start(ShortcutTarget target)
    {
        Cancel();
        dictation.SuspendForRecording();
        Recording = meeting is null ? target : null;
        Waiting = meeting is null ? null : target;
        EndCheck();
        capture = new ShortcutCapture();
        heldKeys.Clear();
        checkedWhileHeld = null;
        messages.Remove(target);
        if (meeting is not null)
        {
            suspensionRef = $"{RefPrefix}pause:{++nextRef}";
            send(new CoreCommand.MeetingsShortcutSuspend(true, suspensionRef));
            var id = suspensionRef;
            timeout = wake.After(CheckTimeout, () =>
            {
                if (suspensionRef != id || Waiting is not ShortcutTarget waiting) return;
                Cancel();
                Show("Couldn't pause the meeting shortcut. The key is unchanged.", waiting);
                Changed();
            });
        }
        Changed();
        Announce($"Recording a shortcut for {Spoken(target)}. Press the keys. Escape on its own cancels.");
    }

    /// <summary>Escape, the button pressed again, the window gone or in the background: nothing changes, and dictation comes back.</summary>
    public void Cancel()
    {
        if (!Busy)
        {
            return;
        }
        Recording = null;
        Waiting = null;
        suspensionRef = null;
        heldKeys.Clear();
        checkedWhileHeld = null;
        EndCheck();
        Resume();
        Changed();
    }

    /// <summary>One key event in the recorder's window. Returns whether the recorder took it (it then goes no further).</summary>
    public bool Feed(ShortcutCapture.Input input)
    {
        if (Capturing is null) return false;
        switch (input)
        {
            case ShortcutCapture.Input.KeyDown down: heldKeys.Add(down.Key); break;
            case ShortcutCapture.Input.KeyUp up: heldKeys.Remove(up.Key); break;
        }
        if (Recording is not ShortcutTarget target)
        {
            if (heldKeys.Count == 0 && checkedWhileHeld is { } answer)
            {
                checkedWhileHeld = null;
                Apply(answer);
            }
            Changed();
            return true;
        }
        switch (capture.Feed(input))
        {
            case ShortcutCapture.Outcome.Cancelled:
                Cancel();
                Announce("Recording cancelled. The key is unchanged.");
                break;
            case ShortcutCapture.Outcome.UnknownKey:
                Show("Inkwell doesn't know that key (keypad and media keys, for one). Try another.", target, ListeningOn);
                Listen(target, heldKeys);
                Changed();
                break;
            case ShortcutCapture.Outcome.Captured captured:
                // AltGr releases as the synthetic left Ctrl; Windows may never send right Alt up.
                if (captured.Token == "right_alt" && input is ShortcutCapture.Input.KeyUp)
                {
                    heldKeys.Clear();
                }
                Recording = null;
                Checking = (target, captured.Token);
                nextRef++;
                var id = $"{RefPrefix}{nextRef}";
                reference = id;
                messages[target] = new ShortcutMessage($"Checking {Describe(captured.Token).Cap}…", false);
                send(new CoreCommand.HotkeyCheck(captured.Token, id));
                timeout = wake.After(CheckTimeout, () => TimedOut(id));
                Changed();
                break;
            default:
                break;
        }
        return true;
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case MeetingsShortcutState state when state.Ref == suspensionRef && suspensionRef is not null && state.Suspended:
                suspensionRef = null;
                timeout?.Dispose();
                timeout = null;
                Recording = Waiting;
                Waiting = null;
                Changed();
                break;
            case CommandFailed failed when failed.Id == suspensionRef && suspensionRef is not null:
                var waiting = Waiting;
                Cancel();
                if (waiting is ShortcutTarget pausedTarget)
                {
                    Show("Couldn't pause the meeting shortcut. The key is unchanged.", pausedTarget);
                }
                Changed();
                break;
            case HotkeyChecked checkedKey when checkedKey.Ref is not null && checkedKey.Ref == reference:
                if (Checking is not var (target, token))
                {
                    return;
                }
                // Do not rebind a key while the recording press is still physically held.
                if (meeting is not null && heldKeys.Count > 0)
                {
                    checkedWhileHeld = checkedKey;
                    return;
                }
                var stillDown = heldKeys.ToList();
                EndCheck();
                // Refused, by the core or here: the next press is the next try.
                if (checkedKey.Ok && checkedKey.Canonical is string canonical)
                {
                    if (Save(canonical, target, Describe(canonical)))
                    {
                        Resume();
                    }
                    else
                    {
                        Listen(target, stillDown);
                    }
                }
                else
                {
                    Show($"Can't use {Describe(token).Cap}: {checkedKey.Reason ?? "Windows can't watch it"}.", target, ListeningOn);
                    Listen(target, stillDown);
                }
                Changed();
                break;
            case CommandFailed failed when failed.Id is not null && failed.Id == reference:
                if (Checking is var (failedTarget, _))
                {
                    EndCheck();
                    Show("Couldn't check that shortcut. The key before still works.", failedTarget);
                }
                Resume();
                Changed();
                break;
            case CoreStopped:
                // Nothing will answer the check now; the restarted core turns dictation on from its
                // switch (DictationModel forgets the pause too).
                if (Busy)
                {
                    Recording = null;
                    Waiting = null;
                    suspensionRef = null;
                    heldKeys.Clear();
                    checkedWhileHeld = null;
                    EndCheck();
                    Changed();
                }
                break;
            default:
                break;
        }
    }

    /// <summary>Whether this model shows a failed command itself.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Id?.StartsWith(RefPrefix, StringComparison.Ordinal) ?? false;
    }

    private void Resume()
    {
        dictation.ResumeAfterRecording();
        if (meeting is not null) send(new CoreCommand.MeetingsShortcutSuspend(false));
    }

    private void TimedOut(string id)
    {
        if (reference != id || Checking is not var (target, _))
        {
            return;
        }
        EndCheck();
        Show("Couldn't check that shortcut in time. The key before still works.", target);
        Resume();
        Changed();
    }

    /// <summary>Forgets the check in flight, and its "Checking…" line.</summary>
    private void EndCheck()
    {
        checkedWhileHeld = null;
        heldKeys.Clear();
        if (Checking is var (target, _) && messages.TryGetValue(target, out var message) && !message.IsProblem)
        {
            messages.Remove(target);
        }
        Checking = null;
        reference = null;
        timeout?.Dispose();
        timeout = null;
    }

    /// <summary>Said after a refusal while the recorder goes on listening.</summary>
    private const string ListeningOn = "Press another key, or Escape to cancel.";

    /// <param name="then">Said after <paramref name="text"/>, not shown: what happens next.</param>
    private void Show(string text, ShortcutTarget target, string? then = null)
    {
        messages[target] = new ShortcutMessage(text, true);
        Announce(then is null ? text : $"{text} {then}");
    }

    /// <summary>
    /// A key was refused: recording goes on for <paramref name="target"/>, dictation still paused,
    /// the refusal still shown. Keys still held from the refused try are ignored until let go of.
    /// </summary>
    private void Listen(ShortcutTarget target, IEnumerable<CapturedKey> stillDown)
    {
        var held = stillDown.ToList();
        Recording = target;
        capture = new ShortcutCapture(held);
        heldKeys.Clear();
        heldKeys.UnionWith(held);
    }

    /// <summary>The two keys are never one: the core would refuse the edit key, and a key that dictates and edits at once does neither well. Whether it saved.</summary>
    private bool Save(string canonical, ShortcutTarget target, DictationKey key)
    {
        if (target != ShortcutTarget.Meeting && meeting?.Key == canonical)
        {
            Show($"{key.Cap} is the meeting key. Pick another, or change the meeting key first.", target, ListeningOn);
            return false;
        }
        switch (target)
        {
            case ShortcutTarget.Meeting:
                if (canonical == dictation.CurrentKey || canonical == dictation.EditKey)
                {
                    Show($"{key.Cap} is a dictation or edit key. Pick another.", target, ListeningOn);
                    return false;
                }
                meeting?.SetKey(canonical);
                break;
            case ShortcutTarget.Dictation:
                if (canonical == dictation.EditKey)
                {
                    Show($"{key.Cap} is the edit key. Pick another, or change the edit key first.", target, ListeningOn);
                    return false;
                }
                dictation.SetKey(canonical);
                break;
            default:
                if (canonical == dictation.CurrentKey)
                {
                    Show($"{key.Cap} is the dictation key. Pick another.", target, ListeningOn);
                    return false;
                }
                saveEditKey(canonical);
                break;
        }
        // The edit key may still wait on the consent step, so it is "chosen", not set.
        var saved = target == ShortcutTarget.Meeting ? $"Meeting key chosen: {key.Name}." : target == ShortcutTarget.Dictation ? $"Dictation key set to {key.Name}." : $"Edit key chosen: {key.Name}.";
        if (KeyNotation.Clash(canonical, key.Cap) is string clash)
        {
            messages[target] = new ShortcutMessage(clash, true);
            Announce($"{saved} {clash}");
        }
        else
        {
            messages.Remove(target);
            Announce(saved);
        }
        return true;
    }
}
