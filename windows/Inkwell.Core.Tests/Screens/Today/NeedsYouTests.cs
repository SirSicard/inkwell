// Today's needs-you banner (as the Mac's NeedsYouTests, and the NeedsYou parts of its
// WatchdogWarningTests, RecoveryNoticeTests, DictationTests and VoiceCommandNoticeTests): what it
// says from the watchdog, the microphone's card, the library's record of meetings that kept no far
// end, and the core's notices. Windows has no system-audio or Accessibility permission to allow.
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.LibraryFixtures;

namespace Inkwell.Core.Tests.Screens;

public class NeedsYouTests
{
    private static readonly LibraryCalendar Utc = new(TimeZoneInfo.Utc, CultureInfo.GetCultureInfo("en-GB"));

    /// <summary>The cards as a permissions check leaves them: <paramref name="off"/> names the ones refused, the rest allowed.</summary>
    private static Func<PermissionName, CardState> Cards(params PermissionName[] off) =>
        p => off.Contains(p) ? CardState.Off : CardState.Allowed;

    private static IReadOnlyList<NeedsYouItem> Items(
        Func<PermissionName, CardState>? permission = null, long farSilent = 0, DateTimeOffset? since = null,
        CoreStore? store = null, FarEndCheck? farEnd = null)
    {
        store ??= new CoreStore();
        return NeedsYou.Items(
            permission ?? Cards(), farEnd ?? new FarEndCheck.Checked(farSilent, since),
            store.Meeting, store.Notices, Utc);
    }

    /// Review fix: the counts that tell whether the far end recorded nothing could not be read.
    /// That is said ("couldn't check"), never read as zero meetings, which would silence the one
    /// warning this banner exists for. Tested through Today's own wiring (NeedsYou.Items from the
    /// permissions, library and store models) from the library model's answers.
    [Fact]
    public void TodaysBannerSaysWhenItCouldNotCheckTheFarEnd()
    {
        var sent = new Sent();
        var library = new LibraryModel(sent.Send, calendar: Utc);
        var permissions = new PermissionsModel(_ => { }, NoCalendar.Instance);
        var store = new CoreStore();
        IReadOnlyList<NeedsYouItem> Today() => NeedsYou.Items(permissions, library, store);
        library.RefreshToday();
        var stats = sent.Commands.Where(c => Cmd(c) == "library.stats").Select(RequestId).ToList();
        Assert.Equal(2, stats.Count);
        Assert.Empty(Today()); // still asking: no guess either way

        foreach (var id in stats)
        {
            library.Apply(Ev.Of($$"""{"type":"command.failed","command":"library.stats","id":"{{id}}","message":"the library: disk I/O error"}"""));
        }
        var failed = Today();
        Assert.Equal(["far-unknown"], failed.Select(i => i.Id));
        Assert.Equal("Inkwell couldn't check the other side of your calls", failed[0].Title);
        Assert.Equal(new NeedsYouAction.RetryChecks(), failed[0].Action);
        Assert.True(library.Handles(Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"library.stats","id":"{{stats[0]}}","message":"x"}""")));

        // Asked again, and answered: two meetings kept no far end.
        sent.Commands.Clear();
        library.RefreshToday();
        var again = sent.Commands.Where(c => Cmd(c) == "library.stats").Select(RequestId).ToList();
        library.Apply(Ev.Of($$"""{"type":"library.stats","ref":"{{again[1]}}","since_unix_ms":0,"kinds":[],"far_silent_meetings":2,"far_silent_since_unix_ms":1789000000000}"""));
        Assert.Equal(["far-silent"], Today().Select(i => i.Id));
    }

    /// Windows: an Allow item's permission is asked for as its Settings card asks.
    [Fact]
    public void AnAllowAsksAsItsSettingsCard()
    {
        Assert.Equal(PermissionCard.HearYou, NeedsYou.Card(PermissionName.Microphone));
        var permissions = new PermissionsModel(_ => { }, NoCalendar.Instance);
        permissions.Apply(Ev.Of("""{"type":"permissions.checked","microphone":"denied","system_audio":"granted","accessibility":"granted","input_monitoring":"granted"}"""));
        var library = new LibraryModel(_ => { }, calendar: Utc);
        Assert.Equal(["perm-mic"], NeedsYou.Items(permissions, library, new CoreStore()).Select(i => i.Id));
    }

    [Fact]
    public void NothingNeedsYouWhenAllIsWell()
    {
        Assert.Empty(Items());
        Assert.Empty(Items(permission: _ => CardState.Checking));
    }

    /// Windows: system audio needs no permission (the check always says granted), so "System audio
    /// is off" and its button show only if a check ever says off; the meetings that kept only the
    /// user's voice carry the same warning, with since when, and open Sound settings.
    [Fact]
    public void SystemAudioOffSaysSinceWhenMeetingsKeptOnlyYourVoice()
    {
        var since = DateTimeOffset.FromUnixTimeSeconds(1_788_000_000); // 29 Aug 2026
        var list = Items(permission: Cards(PermissionName.SystemAudio), farSilent: 4, since: since);
        Assert.Equal(["far-silent", "perm-system-audio"], list.Select(i => i.Id));
        Assert.Equal("System audio is off", list[1].Title);
        Assert.Equal("Open Sound settings", list[1].ActionTitle);
        Assert.Equal(["far-silent"], Items(farSilent: 4, since: since).Select(i => i.Id)); // allowed: no such item
        Assert.Equal("Inkwell didn't hear the other side of your calls", list[0].Title);
        Assert.Equal(
            "Your last 4 meetings (since 29 Aug) kept only your own voice. Check that your calls' sound isn't muted on this PC.",
            list[0].Detail);
        Assert.Equal(new NeedsYouAction.OpenSoundSettings(), list[0].Action);
        Assert.Equal("Open Sound settings", list[0].ActionTitle);
    }

    [Fact]
    public void MeetingsThatKeptNoFarEndRaiseItEvenWhenThePermissionReadsAllowed()
    {
        var list = Items(farSilent: 2);
        Assert.Equal(["far-silent"], list.Select(i => i.Id));
        Assert.StartsWith("Your last 2 meetings kept only your own voice", list[0].Detail, StringComparison.Ordinal);
        Assert.Equal(
            "Your last meeting kept only your own voice. Check that your calls' sound isn't muted on this PC.",
            Items(farSilent: 1)[0].Detail);
    }

    /// Windows: Accessibility is always granted, so no "perm-ax" item follows the microphone's.
    [Fact]
    public void TheWatchdogOnALiveMeetingComesFirst()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.started","record":"r1"}"""),
            Ev.Of("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"""),
            Ev.Of("""{"type":"meeting.side_state","record":"r1","channel":"mic","state":"stopped"}"""),
        ]);
        var list = Items(permission: Cards(PermissionName.Microphone, PermissionName.Accessibility), store: store);
        Assert.Equal(["live-far", "live-mic", "perm-mic"], list.Select(i => i.Id));
        // A silent far end is the plain fact, with no button: nothing on this PC is known to fix it.
        Assert.Equal(("The other side is silent", "Only silence is arriving from the call."), (list[0].Title, list[0].Detail));
        Assert.Null(list[0].Action);
        Assert.Null(list[0].ActionTitle);
        Assert.Equal(new NeedsYouAction.Allow(PermissionName.Microphone), list[1].Action);
        Assert.Equal(new NeedsYouAction.Allow(PermissionName.Microphone), list[2].Action);
    }

    [Fact]
    public void OneOffNoticesAreDismissable()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.warning","record":"r1","kind":"captured_only_zeros","phase":"final"}"""),
            Ev.Of("""{"type":"dictation.hotkey_lost"}"""),
            Ev.Of("""{"type":"model.warmed","id":"m","job":"dictation_final"}"""),
        ]);
        var list = Items(store: store);
        Assert.Equal(
            ["The dictation key stopped working", "A meeting recorded only silence on one side"],
            list.Select(i => i.Title));
        var dismiss = Assert.IsType<NeedsYouAction.Dismiss>(list[0].Action);
        store.DismissNotice(dismiss.NoticeId);
        Assert.Single(Items(store: store));
    }

    /// From the Mac's WatchdogWarningTests: the live warning first, then the new notices.
    [Fact]
    public void TodayListsTheLiveWarningAndTheNewNotices()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}""")]);
        store.Apply([Ev.Of("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}""")]);
        store.Apply([Ev.Of("""{"type":"meeting.recovered","record":"r0","trimmed":1,"rebuilt":0,"unrecoverable":0,"recorded_ms":12000}""")]);
        store.Apply([Ev.Of("""{"type":"meeting.detection","listening":false,"message":"the audio server stopped answering"}""")]);
        var titles = Items(store: store, farEnd: new FarEndCheck.Unknown()).Select(i => i.Title).ToList();
        Assert.Equal("The other side is silent", titles[0]);
        Assert.Contains("A meeting was finished after Inkwell quit unexpectedly", titles);
        Assert.Contains("Inkwell stopped listening for calls", titles);
    }

    /// From the Mac's RecoveryNoticeTests.
    [Fact]
    public void RecoveryThatCouldNotLookIsSaidOnToday()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meetings.recovered","meetings":0}""")]);
        Assert.Empty(store.Notices);
        store.Apply([Ev.Of("""{"type":"meetings.recovered","meetings":0,"message":"couldn't look for meetings a crash interrupted: Not a directory (os error 20)"}""")]);
        Assert.Equal(
            ["Inkwell couldn't check for an unfinished meeting"],
            Items(store: store, farEnd: new FarEndCheck.Unknown()).Select(i => i.Title));
    }

    /// From the Mac's RecoveryNoticeTests.
    [Fact]
    public void AMeetingWithoutItsCrashMarkerSaysItIsNotProtected()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}""")]);
        store.Apply([Ev.Of("""{"type":"meeting.warning","record":"r1","kind":"not_crash_protected","message":"Is a directory (os error 21)"}""")]);
        Assert.Contains(
            "This meeting isn't protected against a crash",
            Items(store: store, farEnd: new FarEndCheck.Unknown()).Select(i => i.Title));
    }

    /// The NeedsYou lines of the Mac's DictationTests (edit key lost) and VoiceCommandNoticeTests
    /// (a command not carried out), whose other assertions are the dictation model's; and the
    /// Windows words where the Mac's name macOS or this Mac.
    [Fact]
    public void KeyLossAndACommandNotCarriedOutAreSaidInWindowsWords()
    {
        Assert.Equal("The edit key stopped working", NeedsYou.Describe(new NoticeKind.EditKeyLost())?.Title);
        Assert.NotNull(NeedsYou.Describe(new NoticeKind.VoiceCommandNotCarriedOut(CommandAction.Undo)));
        var texts = new NoticeKind[]
        {
            new NoticeKind.HotkeyLost(), new NoticeKind.EditKeyLost(), new NoticeKind.LibrarySwept(3, 1),
            new NoticeKind.MeetingWarning(MeetingWarning.CapturedOnlyZeros),
            new NoticeKind.MeetingWarning(MeetingWarning.BluetoothMicOnlyZeros),
        }.Select(k => NeedsYou.Describe(k)!.Value).SelectMany(t => new[] { t.Title, t.Detail }).ToList();
        Assert.DoesNotContain(texts, t => t.Contains("macOS", StringComparison.Ordinal)
            || t.Contains("Mac", StringComparison.Ordinal) || t.Contains("System Settings", StringComparison.Ordinal)
            || t.Contains("Accessibility", StringComparison.Ordinal) || t.Contains("permission", StringComparison.Ordinal));
        Assert.Null(NeedsYou.Describe(new NoticeKind.LibrarySwept(3, 0)));
        Assert.Null(NeedsYou.Describe(new NoticeKind.ModelRefused(Job.DictationFinal)));
    }

    [Fact]
    public void TheBannerShowsOneAndCountsTheRest()
    {
        Assert.Null(NeedsYou.MoreTitle(1, false));
        Assert.Equal("2 more", NeedsYou.MoreTitle(3, false));
        Assert.Equal("Show less", NeedsYou.MoreTitle(3, true));
    }
}
