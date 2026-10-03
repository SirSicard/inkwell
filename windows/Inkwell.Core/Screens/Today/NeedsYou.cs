// Today's "needs you" banner: what the user must act on, most urgent first, as the Mac's NeedsYou.
// Fed by the silent-channel watchdog (a live meeting's side states, and the warnings a meeting
// raised: CoreStore), the permission probes (the Settings screen's cards, passed in), and the
// library's record of recent meetings that kept no far end (LibraryModel's FarEnd): a
// meeting that recorded one side without saying so is what this banner exists to say.
//
// Windows: system audio, Accessibility and input monitoring need no permission (always granted,
// and their permission.request is unsupported), so the Mac's "Allow system audio" and "Allow
// Accessibility" items do not exist here. The far end's silence comes from the watchdog (a live
// meeting's side states, meeting.warning notices) and the library's count, and its button opens
// Windows' Sound settings, where a muted or misrouted call is fixed.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>What a needs-you item's button does.</summary>
public abstract record NeedsYouAction
{
    private NeedsYouAction() { }

    /// <summary>Asks for a permission as its Settings card does: the microphone's opens Settings &gt; Privacy &amp; security &gt; Microphone.</summary>
    public sealed record Allow(PermissionName Permission) : NeedsYouAction;

    /// <summary>Opens Settings &gt; System &gt; Sound (the view launches ms-settings:sound). Windows only: the far end has no permission to allow.</summary>
    public sealed record OpenSoundSettings : NeedsYouAction;

    /// <summary>Dismisses a one-off notice (CoreStore.DismissNotice).</summary>
    public sealed record Dismiss(int NoticeId) : NeedsYouAction;

    /// <summary>Reads Today's counts again (they could not be read).</summary>
    public sealed record RetryChecks : NeedsYouAction;

    /// <summary>Downloads the recommended models (CatalogueModel.DownloadRecommended).</summary>
    public sealed record DownloadModels : NeedsYouAction;
}

/// <summary>One thing that needs the user: what is wrong, and the one thing to do about it, when there is one.</summary>
public sealed record NeedsYouItem(string Id, string Title, string Detail, string? ActionTitle, NeedsYouAction? Action);

public static class NeedsYou
{
    /// <summary>What Today's banner lists, from the models Today reads: the permission cards, the watchdog and notices (CoreStore), and the far-end check (LibraryModel), where "could not read" stays apart from "none" (the Mac's TodayScreen.needItems).</summary>
    public static IReadOnlyList<NeedsYouItem> Items(PermissionsModel permissions, LibraryModel library, CoreStore store, CatalogueModel? catalogue = null)
    {
        ArgumentNullException.ThrowIfNull(permissions);
        ArgumentNullException.ThrowIfNull(library);
        ArgumentNullException.ThrowIfNull(store);
        return Items(
            p => permissions.State(Card(p)), library.FarEnd, store.Meeting, store.Notices, library.Calendar,
            noSpeechModel: catalogue?.HasSpeechModel == false, downloadingModels: catalogue?.Downloading == true);
    }

    /// <summary>The Settings card that asks for <paramref name="permission"/> (an Allow item's button asks as that card does).</summary>
    public static PermissionCard Card(PermissionName permission) =>
        PermissionCards.All.First(card => card.CorePermission() == permission);

    /// <summary>Everything that needs the user now, most urgent first.</summary>
    /// <param name="permission">A permission card's state now (the Settings screen's model).</param>
    /// <param name="calendar">The zone and culture dates are shown in.</param>
    public static IReadOnlyList<NeedsYouItem> Items(
        Func<PermissionName, CardState> permission,
        FarEndCheck farEnd,
        LiveMeeting? meeting,
        IReadOnlyList<Notice> notices,
        LibraryCalendar calendar,
        bool noSpeechModel = false,
        bool downloadingModels = false)
    {
        ArgumentNullException.ThrowIfNull(permission);
        ArgumentNullException.ThrowIfNull(farEnd);
        ArgumentNullException.ThrowIfNull(notices);
        ArgumentNullException.ThrowIfNull(calendar);
        var items = new List<NeedsYouItem>();
        var sound = new NeedsYouAction.OpenSoundSettings();
        var allowMic = new NeedsYouAction.Allow(PermissionName.Microphone);

        // The meeting going on now, as the watchdog judges each side.
        if (meeting is not null)
        {
            switch (meeting.Sides.GetValueOrDefault(Channel.Far, SideState.Ok))
            {
                case SideState.Zeros:
                    // The plain fact, with no button: nothing on this PC is known to fix it.
                    items.Add(new("live-far", "The other side is silent", "Only silence is arriving from the call.", null, null));
                    break;
                case SideState.Stopped:
                    items.Add(new(
                        "live-far", "The other side stopped coming through",
                        "No audio has arrived from the call for a while. This meeting may keep only your voice.",
                        "Check Sound settings", sound));
                    break;
                default:
                    break;
            }
            if (meeting.Sides.GetValueOrDefault(Channel.Mic, SideState.Ok) is SideState.Zeros or SideState.Stopped)
            {
                items.Add(new(
                    "live-mic", "Inkwell can't hear you in this call",
                    "Your microphone is sending silence. This meeting may keep only the other side.",
                    "Check the microphone", allowMic));
            }
        }

        // No speech model: nothing can be written down. One action, the recommended set.
        if (noSpeechModel)
        {
            items.Add(downloadingModels
                ? new("no-speech-model", "No speech model is installed",
                    "The recommended models are downloading. Settings > Models shows how far they are.", null, null)
                : new("no-speech-model", "No speech model is installed",
                    "Nothing you say can be written down until one is. The recommended set is about 640 MB.",
                    "Download recommended models", new NeedsYouAction.DownloadModels()));
        }

        // Whether recent meetings kept the far end, and the microphone's permission.
        switch (farEnd)
        {
            case FarEndCheck.Checked { Meetings: > 0 } check:
                var since = check.Since is { } date ? ShortDay(date, calendar) : null;
                var count = check.Meetings == 1 ? "Your last meeting" : $"Your last {check.Meetings} meetings";
                var from = check.Meetings > 1 && since is not null ? $" (since {since})" : "";
                items.Add(new(
                    "far-silent", "Inkwell didn't hear the other side of your calls",
                    $"{count}{from} kept only your own voice. Check that your calls' sound isn't muted on this PC.",
                    "Open Sound settings", sound));
                break;
            case FarEndCheck.Failed:
                // Not known is not "none": the warning this banner exists for may be the one hidden.
                items.Add(new(
                    "far-unknown", "Inkwell couldn't check the other side of your calls",
                    "Your library could not be read to see whether recent meetings kept both sides.",
                    "Try again", new NeedsYouAction.RetryChecks()));
                break;
            default:
                break;
        }
        // Only when the check says so (on Windows it never does: system audio needs no permission).
        if (permission(PermissionName.SystemAudio) == CardState.Off)
        {
            items.Add(new(
                "perm-system-audio", "System audio is off",
                "Meetings record only your voice.",
                "Open Sound settings", sound));
        }
        if (permission(PermissionName.Microphone) == CardState.Off)
        {
            items.Add(new(
                "perm-mic", "Inkwell can't hear you",
                "Microphone access is off, so dictation and meetings record nothing from you.",
                "Allow the microphone", allowMic));
        }

        // One-off notices the core sent: what went wrong in a meeting or with the dictation key.
        for (var i = notices.Count - 1; i >= 0; i--)
        {
            var notice = notices[i];
            if (Describe(notice.Kind) is { } text)
            {
                items.Add(new(
                    $"notice-{notice.Id}", text.Title, text.Detail, "Dismiss", new NeedsYouAction.Dismiss(notice.Id)));
            }
        }
        return items;
    }

    /// <summary>The notices that need the user, in words; null for the rest (the screens that own them show them: Settings, Live).</summary>
    public static (string Title, string Detail)? Describe(NoticeKind kind) => kind switch
    {
        NoticeKind.MeetingWarning { Warning: MeetingWarning.CapturedOnlyZeros } =>
            ("A meeting recorded only silence on one side", "One side of the last meeting arrived as digital silence: it was probably muted."),
        NoticeKind.MeetingWarning { Warning: MeetingWarning.BluetoothMicOnlyZeros } =>
            ("Your headset's microphone sent only silence", "Bluetooth headset mics can go silent in calls. Inkwell records another mic instead when it can."),
        NoticeKind.MeetingWarning { Warning: MeetingWarning.NothingCaptured } =>
            ("The last meeting recorded nothing", "No audio reached Inkwell from either side."),
        NoticeKind.MeetingWarning { Warning: MeetingWarning.NotCrashProtected } =>
            ("This meeting isn't protected against a crash", "If Inkwell quits unexpectedly, it won't finish this meeting at the next launch. The recording is still being saved."),
        NoticeKind.MeetingWarning { Warning: MeetingWarning.FarEndQuietWhileYouSpeak } =>
            ("The other side went quiet while you spoke", "Their audio may not be reaching Inkwell."),
        NoticeKind.MeetingFailed or NoticeKind.MeetingCaptureFailed or NoticeKind.MeetingWorkerFailed =>
            ("The last meeting stopped early", "What was recorded up to then is kept."),
        NoticeKind.HotkeyLost =>
            ("The dictation key stopped working", "Windows stopped sending it to Inkwell. Turn dictation on again to get it back."),
        NoticeKind.MeetingRecovered =>
            ("A meeting was finished after Inkwell quit unexpectedly", "It was recording when Inkwell stopped. What was recorded was kept and the record is complete."),
        NoticeKind.RecoveryUnavailable =>
            ("Inkwell couldn't check for an unfinished meeting", "If a meeting was recording when Inkwell last quit, it may not be finished yet. Inkwell looks again at the next launch."),
        NoticeKind.DetectionUnavailable =>
            ("Inkwell stopped listening for calls", "It can't tell when a call starts, so it won't offer to record one. Record now still works."),
        NoticeKind.LibrarySwept { Failed: > 0 } =>
            ("Some old records couldn't be removed", "Your storage setting deletes old records, and some of them, or their recordings, are still on this PC."),
        NoticeKind.VoiceCommandNotCarriedOut =>
            ("Inkwell heard a voice command it can’t do yet", "That command isn’t available in this version, so nothing was typed. Settings > Voice commands shows which ones work."),
        NoticeKind.EditKeyLost =>
            ("The edit key stopped working", "Windows stopped sending it to Inkwell, so editing a selection by voice is off. Turn dictation on again to get it back."),
        _ => null,
    };

    /// <summary>The banner's toggle under its first item: "2 more", "Show less", or null with one item.</summary>
    public static string? MoreTitle(int count, bool showingAll) =>
        count <= 1 ? null : showingAll ? "Show less" : $"{count - 1} more";

    /// <summary>A day and abbreviated month in the locale's order: "29 Aug" (en-GB), "Aug 29" (en-US).</summary>
    internal static string ShortDay(DateTimeOffset date, LibraryCalendar calendar)
    {
        var pattern = calendar.Culture.DateTimeFormat.MonthDayPattern.Replace("MMMM", "MMM", StringComparison.Ordinal);
        return calendar.Clock(date).ToString(pattern, calendar.Culture);
    }
}
