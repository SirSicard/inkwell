// The permission cards: what each shows for the core's check, when checks run, and what a card's
// button asks for on Windows (the Mac's PermissionsModelTests).
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

file sealed class FakeCalendar(CardState answer) : ICalendarAccess
{
    public int Requests { get; private set; }

    public CardState State() => answer;

    public void Request(Action done)
    {
        Requests++;
        done();
    }
}

file static class PermissionEvents
{
    /// <summary>permissions.checked as the Windows core sends it: only the microphone can be anything but granted.</summary>
    public static InkEvent Checked(string mic = "granted", string system = "granted", string ax = "granted") =>
        Ev.Of($$"""{"type":"permissions.checked","microphone":"{{mic}}","system_audio":"{{system}}","accessibility":"{{ax}}","input_monitoring":"granted"}""");
}

/// <summary>
/// A core whose probe answers each check 900 ms after it was sent (the Mac test's probe time), with
/// what the probe reads when the check was sent, on a clock the test moves (nothing here sleeps).
/// </summary>
file sealed class FakeProbeCore
{
    private readonly List<(TimeSpan Due, InkEvent Answer)> pending = [];

    public string Microphone { get; set; } = "granted";
    public int Checks { get; private set; }
    public PermissionsModel? Model { get; set; }
    public TimeSpan Now { get; private set; }

    public void Send(CoreCommand command)
    {
        if (command is not CoreCommand.PermissionsCheck)
        {
            return;
        }
        Checks++;
        pending.Add((Now + TimeSpan.FromMilliseconds(900), PermissionEvents.Checked(mic: Microphone)));
    }

    /// <summary>Moves the clock in 20 ms steps until <paramref name="done"/> or <paramref name="timeout"/>, delivering answers as they fall due.</summary>
    public void RunUntil(Func<bool> done, TimeSpan timeout)
    {
        var start = Now;
        while (!done())
        {
            Assert.True(Now - start <= timeout, $"not within {timeout}");
            Now += TimeSpan.FromMilliseconds(20);
            foreach (var answer in pending.Where(p => p.Due <= Now).ToList())
            {
                pending.Remove(answer);
                Model!.Apply(answer.Answer);
            }
        }
    }
}

public class PermissionsModelTests
{
    /// <summary>
    /// Verify: revoking a permission turns its card red within 5 s of coming back. The Mac test
    /// revokes system audio (RevokingSystemAudioTurnsItsCardRedWithinFiveSecondsOfComingBack); on
    /// Windows only the microphone can be revoked, so this revokes it in Settings: coming back to
    /// the app re-checks, and the answer turns the card red.
    /// </summary>
    [Fact]
    public void RevokingTheMicrophoneTurnsItsCardRedWithinFiveSecondsOfComingBack()
    {
        var core = new FakeProbeCore();
        var model = new PermissionsModel(core.Send, new FakeCalendar(CardState.Allowed));
        core.Model = model;
        model.ScreenAppeared();
        core.RunUntil(() => model.State(PermissionCard.HearYou) == CardState.Allowed, TimeSpan.FromSeconds(5));
        Assert.False(model.State(PermissionCard.HearYou).IsAlert());

        // Turned off in Settings > Privacy & security > Microphone; the user comes back to Inkwell.
        core.Microphone = "denied";
        var back = core.Now;
        model.AppBecameActive();
        core.RunUntil(() => model.State(PermissionCard.HearYou) == CardState.Off, TimeSpan.FromSeconds(5));
        Assert.True(core.Now - back < TimeSpan.FromSeconds(5));
        Assert.True(model.State(PermissionCard.HearYou).IsAlert(), "red");
        Assert.Equal([PermissionCard.HearYou], model.OffCards);
        Assert.Equal("The microphone is off. Nothing you say can be written down.", PermissionCard.HearYou.OffDetail());
        Assert.Equal(2, core.Checks);
    }

    [Fact]
    public void ChecksRunOnlyWhenAScreenShowsTheCardsOrAfterLaunchNeverOnATimer()
    {
        var sent = new Sent();
        var model = new PermissionsModel(sent.Send, new FakeCalendar(CardState.NotAsked));
        model.AppBecameActive();
        Assert.Empty(sent.Commands); // no card on screen: activation checks nothing
        model.ScreenAppeared();
        Assert.Equal([new CoreCommand.PermissionsCheck()], sent.Commands);
        model.AppBecameActive();
        Assert.Equal([new CoreCommand.PermissionsCheck(), new CoreCommand.PermissionsCheck()], sent.Commands);
        model.ScreenDisappeared();
        model.AppBecameActive();
        Assert.Equal(2, sent.Commands.Count);
    }

    [Fact]
    public void AFailedCheckLeavesNoCardGreenOrRed()
    {
        var model = new PermissionsModel(_ => { }, new FakeCalendar(CardState.Allowed));
        model.Refresh();
        model.Apply(PermissionEvents.Checked(mic: "granted", system: "denied", ax: "granted"));
        Assert.Equal(CardState.Allowed, model.State(PermissionCard.HearYou));
        model.Refresh();
        model.Apply(Ev.Of("""{"type":"command.failed","command":"permissions.check","message":"a bug in the core stopped this command"}"""));
        foreach (var card in new[] { PermissionCard.HearYou, PermissionCard.HearTheOthers, PermissionCard.TypeForYou })
        {
            Assert.Equal(CardState.Unknown, model.State(card)); // neither green nor red after a failed check
        }
        Assert.Equal(CardState.Allowed, model.State(PermissionCard.KnowYourMeetings)); // the calendar is read by the shell, not the check
        Assert.False(model.Checking);
        Assert.True(PermissionsModel.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"permissions.check","message":"x"}""")));
    }

    /// <summary>
    /// Windows: the Mac's system-audio request is gone (Windows has none to ask for), so a press on
    /// that card sends nothing; the microphone's asks the core, which opens Settings.
    /// </summary>
    [Fact]
    public void EachStateReadsAsItsCardAndAllowAsksTheCoreOrTheCalendar()
    {
        var sent = new Sent();
        var calendar = new FakeCalendar(CardState.Off);
        var model = new PermissionsModel(sent.Send, calendar);
        Assert.Equal(CardState.Checking, model.State(PermissionCard.HearYou));
        model.Refresh();
        model.Apply(PermissionEvents.Checked(mic: "not_determined", system: "unknown", ax: "denied"));
        Assert.Equal(CardState.NotAsked, model.State(PermissionCard.HearYou));
        Assert.Equal(CardState.Unknown, model.State(PermissionCard.HearTheOthers));
        Assert.Equal(CardState.Off, model.State(PermissionCard.TypeForYou));
        Assert.Equal(CardState.Off, model.State(PermissionCard.KnowYourMeetings)); // the calendar's answer, read by the shell
        var before = sent.Commands.Count;
        model.Request(PermissionCard.HearTheOthers);
        Assert.Equal(before, sent.Commands.Count);
        model.Request(PermissionCard.HearYou);
        Assert.Equal(new CoreCommand.PermissionRequest(PermissionName.Microphone), sent.Commands[^1]);
        model.Request(PermissionCard.KnowYourMeetings);
        Assert.Equal(1, calendar.Requests);
        Assert.IsType<CoreCommand.PermissionsCheck>(sent.Commands[^1]); // done: checked again
        Assert.Equal(
            ["Hear you", "Hear the others", "Type for you", "Know your meetings"],
            PermissionCards.All.Select(c => c.Title()));
    }

    /// <summary>
    /// Windows: the cards the core always answers granted read as allowed, say why, and never
    /// offer a button; the microphone's button opens Settings (no prompt exists); the calendar of
    /// this build says it is not available, never refused, and asks nothing.
    /// </summary>
    [Fact]
    public void OnWindowsNoCardOffersARequestThatCannotWork()
    {
        var sent = new Sent();
        var model = new PermissionsModel(sent.Send);
        model.Refresh();
        model.Apply(PermissionEvents.Checked(mic: "denied"));
        Assert.Equal(CardState.Allowed, model.State(PermissionCard.HearTheOthers));
        Assert.Equal(CardState.Allowed, model.State(PermissionCard.TypeForYou));
        Assert.Null(PermissionCard.HearTheOthers.ActionTitle(CardState.Allowed));
        Assert.Contains("Windows asks no permission", PermissionCard.HearTheOthers.Detail(), StringComparison.Ordinal);
        Assert.Contains("run as administrator", PermissionCard.TypeForYou.Detail(), StringComparison.Ordinal);
        // Even a card the core could not check offers no request Windows does not have.
        Assert.Null(PermissionCard.TypeForYou.ActionTitle(CardState.Unknown));
        Assert.Null(PermissionCard.HearTheOthers.ActionTitle(CardState.Off));

        Assert.Equal("Open Settings", PermissionCard.HearYou.ActionTitle(model.State(PermissionCard.HearYou)));
        Assert.Equal("Open Settings", PermissionCard.HearYou.ActionTitle(CardState.NotAsked));
        Assert.Null(PermissionCard.HearYou.ActionTitle(CardState.Allowed));

        Assert.Equal(CardState.Unavailable, model.State(PermissionCard.KnowYourMeetings));
        Assert.Null(PermissionCard.KnowYourMeetings.ActionTitle(CardState.Unavailable));
        Assert.Equal(PermissionCards.CalendarUnavailableDetail, PermissionCard.KnowYourMeetings.Line(CardState.Unavailable));
        Assert.Equal("Not available", PermissionCards.StateLabel(CardState.Unavailable));
        Assert.False(CardState.Unavailable.IsAlert());
        var before = sent.Commands.Count;
        model.Request(PermissionCard.KnowYourMeetings);
        model.Request(PermissionCard.TypeForYou);
        Assert.Equal(before, sent.Commands.Count);
        Assert.Equal([PermissionCard.HearYou], model.OffCards); // not available is not off

        // A calendar a packaged build can reach asks as on the Mac.
        Assert.Equal("Allow", PermissionCard.KnowYourMeetings.ActionTitle(CardState.NotAsked));
        Assert.Equal("Open Settings", PermissionCard.KnowYourMeetings.ActionTitle(CardState.Unknown));
    }
}
