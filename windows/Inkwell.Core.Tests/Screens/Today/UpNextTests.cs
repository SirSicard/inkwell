// Today's Up next (as the Mac's UpNextTests): the calendar's next meeting, asked for only when the
// user connects, its "in 42 min", and that its minute redraw runs only while the window is on
// screen. The calendar here is a fake; this Windows build's real one is NoCalendar.
using System.Globalization;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class UpNextTests
{
    /// <summary>The calendar's permission, as the test sets it.</summary>
    private sealed class FakeCalendarAccess : ICalendarAccess
    {
        public CardState Current { get; set; } = CardState.NotAsked;
        public bool Grant { get; set; } = true;
        public int Asked { get; private set; }

        public CardState State() => Current;

        public void Request(Action done)
        {
            Asked++;
            Current = Grant ? CardState.Allowed : CardState.Off;
            done();
        }
    }

    /// <summary>The calendar's events, as the test sets them.</summary>
    private sealed class FakeEvents(FakeCalendarAccess access) : IUpcomingEvents
    {
        public List<UpcomingEvent> Events { get; set; } = [];

        public UpcomingEvent? NextEvent(DateTimeOffset now, TimeSpan horizon) =>
            access.Current != CardState.Allowed ? null
            : Events.Where(e => e.Start > now && e.Start < now + horizon).MinBy(e => e.Start);
    }

    /// <summary>Wakes the test runs by hand.</summary>
    private sealed class FakeWakes : IWakeScheduler
    {
        public List<(TimeSpan Delay, Action Wake, Pending Handle)> Scheduled { get; } = [];

        public IDisposable After(TimeSpan delay, Action wake)
        {
            var handle = new Pending();
            Scheduled.Add((delay, wake, handle));
            return handle;
        }

        public IEnumerable<(TimeSpan Delay, Action Wake, Pending Handle)> Live => Scheduled.Where(s => !s.Handle.Cancelled);

        public sealed class Pending : IDisposable
        {
            public bool Cancelled { get; private set; }

            public void Dispose() => Cancelled = true;
        }
    }

    private static readonly DateTimeOffset Now = DateTimeOffset.FromUnixTimeSeconds(1_790_000_000);

    private static (UpNextModel Model, FakeCalendarAccess Access, FakeEvents Events, FakeWakes Wakes) Model(bool grant = true)
    {
        var access = new FakeCalendarAccess { Grant = grant };
        var events = new FakeEvents(access)
        {
            Events = [new UpcomingEvent("Pilot check-in", Now.AddSeconds(2_520), Now.AddSeconds(4_320), "Zoom")],
        };
        var wakes = new FakeWakes();
        return (new UpNextModel(access, events, wakes, () => Now), access, events, wakes);
    }

    [Fact]
    public void NothingIsReadOrAskedUntilTheUserConnects()
    {
        var (model, access, _, _) = Model();
        model.Refresh();
        Assert.Equal(CardState.NotAsked, model.Access);
        Assert.Null(model.Event);
        Assert.Equal(0, access.Asked);
        Assert.Equal("See your next meeting here.", model.Note);
        Assert.Equal("Show my next meeting", model.ConnectTitle);

        model.Connect();
        Assert.Equal(1, access.Asked);
        Assert.Equal(CardState.Allowed, model.Access);
        Assert.Equal("Pilot check-in", model.Event?.Title);
        Assert.Equal("in 42 min", MeetingApp.StartsIn(model.Event!.Start, Now));
        Assert.Null(model.Note);
        Assert.Null(model.ConnectTitle);
        Assert.Equal("14:55 · Zoom · in 42 min", model.MetaLine(Now, new LibraryCalendar(TimeZoneInfo.Utc, CultureInfo.GetCultureInfo("en-GB"))));
        Assert.Equal("Records when Zoom opens the microphone", model.RecordsWhen);
    }

    [Fact]
    public void ADeniedCalendarShowsNoEvent()
    {
        var (model, _, _, _) = Model(grant: false);
        model.Connect();
        Assert.Equal(CardState.Off, model.Access);
        Assert.Null(model.Event);
        Assert.Equal("Calendar access is off, so Inkwell can't show your next meeting.", model.Note);
        Assert.Equal("Open Calendar settings", model.ConnectTitle);
    }

    /// Windows only: this unpackaged build has no calendar. Up next says so, offers nothing to
    /// ask, and never reads as an empty calendar or a refusal.
    [Fact]
    public void WithNoCalendarUpNextSaysItIsNotAvailable()
    {
        var wakes = new FakeWakes();
        var model = new UpNextModel(NoCalendar.Instance, NoCalendar.Instance, wakes, () => Now);
        model.Refresh();
        Assert.Equal(CardState.Unavailable, model.Access);
        Assert.Null(model.Event);
        Assert.Equal("Your calendar isn't available in this version of Inkwell, so it can't show your next meeting.", model.Note);
        Assert.Null(model.ConnectTitle);
        Assert.False(model.Ticks(onScreen: true));
        Assert.Empty(wakes.Scheduled);
    }

    /// The architecture rule: nothing ticks while idle. "in N min" redraws on the minute only
    /// while an event is shown and the window is on screen; paused, its schedule gives one entry
    /// (the frame drawn when it stopped) and then none.
    [Fact]
    public void TheMinuteTicksOnlyWhileAnEventIsShownOnScreen()
    {
        var (model, _, _, _) = Model();
        model.Connect();
        Assert.True(model.Ticks(onScreen: true));
        Assert.False(model.Ticks(onScreen: false), "hidden, occluded or minimised");

        var start = DateTimeOffset.FromUnixTimeSeconds(1_000_030); // 30 s past a minute
        var running = MinuteSchedule.Entries(start, paused: false).Take(3).Select(d => d.ToUnixTimeSeconds());
        Assert.Equal([1_000_030L, 1_000_080L, 1_000_140L], running);
        Assert.Equal([start], MinuteSchedule.Entries(start, paused: true).Take(3));

        var (empty, _, events, _) = Model();
        events.Events = [];
        empty.Connect();
        Assert.False(empty.Ticks(onScreen: true), "no event, nothing to count down");
        Assert.Equal("Nothing else on your calendar today.", empty.Note);
    }

    /// Windows: the Mac's one-shot Task.sleep to the event's start is an injected wake; a refresh
    /// replaces it, so only one is ever pending, and it moves on to the next event.
    [Fact]
    public void UpNextMovesOnWithOneWakeAtTheStartNotATimer()
    {
        var (model, _, events, wakes) = Model();
        model.Connect();
        var wake = Assert.Single(wakes.Live);
        Assert.Equal(TimeSpan.FromSeconds(2_521), wake.Delay);

        model.Refresh();
        Assert.Single(wakes.Live);
        Assert.Equal(2, wakes.Scheduled.Count);

        events.Events = [new UpcomingEvent("Design review", Now.AddHours(2), Now.AddHours(3), null)];
        wakes.Live.Single().Wake();
        Assert.Equal("Design review", model.Event?.Title);
        Assert.Equal("Records when the call opens the microphone", model.RecordsWhen);
        Assert.Single(wakes.Live);

        events.Events = [];
        wakes.Live.Single().Wake();
        Assert.Null(model.Event);
        Assert.Empty(wakes.Live);
    }

    [Fact]
    public void TheWindowIsOnScreenOnlyVisibleUncoveredAndNotMinimised()
    {
        Assert.True(WindowPresence.IsOnScreen(visible: true, minimized: false, occlusionVisible: true));
        Assert.False(WindowPresence.IsOnScreen(visible: true, minimized: false, occlusionVisible: false));
        Assert.False(WindowPresence.IsOnScreen(visible: true, minimized: true, occlusionVisible: true));
        Assert.False(WindowPresence.IsOnScreen(visible: false, minimized: false, occlusionVisible: true));
        var presence = new WindowPresence();
        Assert.False(presence.OnScreen, "no window");
        presence.Update(visible: true, minimized: false, occlusionVisible: true);
        Assert.True(presence.OnScreen);
        presence.Update(visible: true, minimized: true, occlusionVisible: true);
        Assert.False(presence.OnScreen);
    }

    [Fact]
    public void TheMeetingAppComesFromTheLinkPlaceOrNotes()
    {
        Assert.Equal("Zoom", MeetingApp.Name(new Uri("https://us02web.zoom.us/j/123"), null, null));
        Assert.Equal("Teams", MeetingApp.Name(null, "Microsoft Teams Meeting", "Join: https://teams.microsoft.com/l/meetup-join/x"));
        Assert.Equal("Meet", MeetingApp.Name(null, null, "meet.google.com/abc-defg-hij"));
        Assert.Null(MeetingApp.Name(null, "Room 4", null));
    }

    [Fact]
    public void StartsInReadsInMinutesAndHours()
    {
        Assert.Equal("in 1 min", MeetingApp.StartsIn(Now.AddSeconds(30), Now));
        Assert.Equal("in 1 h 5 min", MeetingApp.StartsIn(Now.AddSeconds(3_900), Now));
        Assert.Equal("in 2 h", MeetingApp.StartsIn(Now.AddSeconds(7_200), Now));
        Assert.Equal("now", MeetingApp.StartsIn(Now.AddSeconds(-10), Now));
    }
}
