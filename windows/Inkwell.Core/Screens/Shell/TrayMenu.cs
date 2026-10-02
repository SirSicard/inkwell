// The tray icon's menu and state, as the Mac's menu-bar item: a status line, Record Now or Stop
// Recording, Dictation, Open Inkwell, Settings…, Start with Windows, Check for Updates…, Quit. The
// menu is made when it opens (right-click), so its status line is read then and nothing ticks.
// The icon shows the state: idle, dictating (a dot in your colour), recording (a dot in theirs),
// or a problem (the alert colour).
namespace Inkwell.Core.Screens;

/// <summary>What the tray icon shows.</summary>
public enum TrayState
{
    Idle,
    Dictating,
    Recording,
    Problem,
}

public static class TrayMenu
{
    public const string RecordNowTitle = "Record Now";
    public const string StopTitle = "Stop Recording";
    public const string DictationTitle = "Dictation";
    public const string OpenTitle = "Open Inkwell";
    public const string SettingsTitle = "Settings…";
    public const string CheckForUpdatesTitle = "Check for Updates…";
    public const string QuitTitle = "Quit Inkwell";

    /// <summary>The icon's state for what the Drop's ink shows.</summary>
    public static TrayState State(DropInk ink) => ink switch
    {
        DropInk.Dictating => TrayState.Dictating,
        DropInk.Meeting or DropInk.Blotting => TrayState.Recording,
        DropInk.Problem => TrayState.Problem,
        _ => TrayState.Idle,
    };

    /// <summary>
    /// The menu's first line: "Ready", "Dictating", "Recording · Design review · 12:04",
    /// "Blotting · Design review", or the core's state while it is not ready.
    /// </summary>
    /// <param name="elapsedMs">How long the meeting has recorded, when known (LiveModel.ElapsedMs).</param>
    public static string StatusLine(CoreStore store, long? elapsedMs)
    {
        ArgumentNullException.ThrowIfNull(store);
        if (store.Status.Kind != CoreStatusKind.Ready)
        {
            return store.Status.Kind switch
            {
                CoreStatusKind.Starting => "Starting",
                CoreStatusKind.Stopped => "The core stopped",
                _ => "The core did not start",
            };
        }
        if (store.Meeting is { } meeting)
        {
            var name = meeting.Title ?? meeting.AppName;
            if (meeting.Stopping)
            {
                return name is null ? "Blotting" : $"Blotting · {name}";
            }
            var parts = new List<string> { "Recording" };
            if (name is not null)
            {
                parts.Add(name);
            }
            if (elapsedMs is long ms)
            {
                parts.Add(LiveClock.Of(ms));
            }
            return string.Join(" · ", parts);
        }
        return store.Dictation == DictationPhase.Idle ? "Ready" : "Dictating";
    }

    /// <summary>Record Now while nothing records, Stop Recording while a meeting records (not while it blots).</summary>
    public static (string Title, bool Enabled) RecordItem(CoreStore store)
    {
        ArgumentNullException.ThrowIfNull(store);
        return store.Meeting switch
        {
            null => (RecordNowTitle, store.Status.Kind == CoreStatusKind.Ready),
            { Stopping: true } => (StopTitle, false),
            _ => (StopTitle, true),
        };
    }
}
