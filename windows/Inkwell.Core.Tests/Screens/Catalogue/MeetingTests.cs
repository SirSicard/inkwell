// Meetings in the shell (as the Mac's MeetingsTests): Settings > Meetings and Storage's retention,
// Record now, the Drop's answers and Stop with their failure lines, and the recovery a launch asks
// for.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

internal sealed class FixedTitle(string? title) : ICallTitles
{
    public string? TitleNow(DateTimeOffset now) => title;
}

public class MeetingSettingsTests
{
    [Fact]
    public void TheSettingsReadAndWriteTheCoresKeys()
    {
        var sent = new Sent();
        var meetings = new MeetingModel(sent.Send);
        meetings.Load();
        // The old "Offer to record calls" switch is the call policies' default now (CallPolicyModel).
        Assert.Equal(
            [new CoreCommand.SettingGet(ShellSetting.MeetingsHeadsetMic), new CoreCommand.SettingGet(ShellSetting.RetentionDays)],
            sent.Commands);
        Assert.Null(meetings.Retention); // not known until the core answers
        meetings.Apply(Ev.Of("""{"type":"setting.value","key":"meetings.headset_mic","value":"on"}"""));
        meetings.Apply(Ev.Of("""{"type":"setting.value","key":"retention.days"}"""));
        Assert.True(meetings.HeadsetMic);
        Assert.Equal(Retention.Forever, meetings.Retention); // never set: forever
        meetings.SetRetention(Retention.Month);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.RetentionDays, "30"), sent.Commands[^1]);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:retention.days","message":"x"}""");
        meetings.Apply(failed);
        Assert.True(meetings.SettingsFailed);
        Assert.True(MeetingModel.Handles(failed));
        Assert.False(MeetingModel.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:dictation.key","message":"x"}""")));
        Assert.Equal(["forever", "7", "30", "90", "365"], Retentions.All.Select(r => r.Value())); // the core's whitelist
        meetings.Apply(Ev.Of("""{"type":"setting.value","key":"retention.days","value":"90"}"""));
        Assert.Equal(Retention.Quarter, meetings.Retention);
    }
}

public class MeetingFailureTests
{
    /// <summary>
    /// Review (S2.8): a failed Record now shows where it was pressed (Today, and Live's Record now),
    /// and nowhere else; it is logged by command name, never with the core's words.
    /// </summary>
    [Fact]
    public void ARecordNowFailureShowsWhereItWasPressed()
    {
        var sent = new Sent();
        var logged = new Logged();
        var meetings = new MeetingModel(sent.Send, new FixedTitle(null), log: logged.Log);
        meetings.RecordNow();
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}""");
        meetings.Apply(failed);
        Assert.Equal("Couldn't start recording: the microphone: no input device", meetings.FailureOn(MeetingPlace.RecordNow));
        Assert.Null(meetings.FailureOn(MeetingPlace.LiveStop));
        Assert.Equal(["command.failed for a meeting.start command; shown where it was asked"], logged.Messages);
        Assert.True(MeetingModel.Handles(failed));
    }

    /// <summary>Live's Stop: a failed stop shows in Live's header, beside the button.</summary>
    [Fact]
    public void AStopThatFailsShowsInLive()
    {
        var sent = new Sent();
        var meetings = new MeetingModel(sent.Send, new FixedTitle(null), log: new Logged().Log);
        meetings.Stop();
        meetings.Apply(Ev.Of("""{"type":"command.failed","command":"meeting.stop","id":"meeting.stop","message":"the meeting is already stopping"}"""));
        Assert.Equal("Couldn't stop: the meeting is already stopping", meetings.FailureOn(MeetingPlace.LiveStop));
        Assert.Null(meetings.FailureOn(MeetingPlace.RecordNow));
    }
}

public class RecordNowTests
{
    [Fact]
    public void RecordNowNamesTheCallFromTheCalendarAndAFailureIsSaid()
    {
        var sent = new Sent();
        var meetings = new MeetingModel(sent.Send, new FixedTitle(null), log: new Logged().Log);
        meetings.RecordNow();
        Assert.Equal([new CoreCommand.MeetingStart(null, null)], sent.Commands);
        meetings.Apply(Ev.Of("""{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}"""));
        Assert.Equal("Couldn't start recording: the microphone: no input device", meetings.FailureOn(MeetingPlace.RecordNow));
        meetings.Apply(Ev.Of("""{"type":"meeting.started","record":"r1"}"""));
        Assert.Null(meetings.FailureOn(MeetingPlace.RecordNow));
        meetings.Stop();
        Assert.Equal(new CoreCommand.MeetingStop(), sent.Commands[^1]);

        // Windows: with a calendar that has the call, the start carries its title; this build's (NoCalendar) has none.
        var titled = new Sent();
        new MeetingModel(titled.Send, new FixedTitle("Weekly sync")).RecordNow();
        Assert.Equal([new CoreCommand.MeetingStart(null, "Weekly sync")], titled.Commands);
        var none = new Sent();
        new MeetingModel(none.Send).RecordNow();
        Assert.Equal([new CoreCommand.MeetingStart(null, null)], none.Commands);
    }
}

public class DropAnswerTests
{
    /// <summary>"Record this call" starts the offered app's meeting (titled from the calendar when it has the call); "Not this one" dismisses it.</summary>
    [Fact]
    public void TheDropsAnswersNameTheOfferedApp()
    {
        var sent = new Sent();
        var meetings = new MeetingModel(sent.Send, new FixedTitle("Weekly sync"), log: new Logged().Log);
        meetings.Record("ms-teams.exe");
        meetings.Dismiss("Zoom.exe");
        meetings.Perform(new DropAction.Record("chrome.exe"));
        meetings.Perform(new DropAction.Dismiss("chrome.exe"));
        Assert.Equal(
            [
                new CoreCommand.MeetingStart("ms-teams.exe", "Weekly sync"), new CoreCommand.MeetingDismiss("Zoom.exe"),
                new CoreCommand.MeetingStart("chrome.exe", "Weekly sync"), new CoreCommand.MeetingDismiss("chrome.exe"),
            ],
            sent.Commands);
    }

    /// <summary>A failed answer shows on the Drop and nowhere else; Record now's shows where it was pressed.</summary>
    [Fact]
    public void AFailedAnswerShowsOnTheDropOnly()
    {
        var sent = new Sent();
        var meetings = new MeetingModel(sent.Send, new FixedTitle(null), log: new Logged().Log);
        // The offer the answers answer: a failure belongs to it.
        meetings.Apply(Ev.Of("""{"type":"meeting.detected","app":"ms-teams.exe","app_name":"ms-teams"}"""));
        meetings.Record("ms-teams.exe");
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}""");
        meetings.Apply(failed);
        Assert.Equal("Couldn't start recording: the microphone: no input device", meetings.FailureOn(MeetingPlace.Drop));
        Assert.Null(meetings.FailureOn(MeetingPlace.RecordNow));
        Assert.Null(meetings.FailureOn(MeetingPlace.LiveStop));

        meetings.Dismiss("ms-teams.exe");
        Assert.Null(meetings.FailureOn(MeetingPlace.Drop));
        var dismissFailed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"meeting.dismiss","id":"meeting.dismiss","message":"that app is not being offered"}""");
        Assert.True(MeetingModel.Handles(dismissFailed));
        meetings.Apply(dismissFailed);
        Assert.Equal("Couldn't dismiss the offer: that app is not being offered", meetings.FailureOn(MeetingPlace.Drop));

        meetings.RecordNow();
        meetings.Apply(failed);
        Assert.Null(meetings.FailureOn(MeetingPlace.Drop));
        Assert.NotNull(meetings.FailureOn(MeetingPlace.RecordNow));
    }

    [Fact]
    public void RecoveryIsAskedForByName()
    {
        var sent = new Sent();
        new MeetingModel(sent.Send).Recover();
        var recover = Assert.Single(sent.Commands);
        Assert.Equal("meetings.recover", recover.Name);
        Assert.IsType<CoreCommand.MeetingsRecover>(recover);
    }
}
