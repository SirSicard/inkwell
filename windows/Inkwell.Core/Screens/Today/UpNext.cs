// Today's "Up next": the next meeting on the user's calendars, as the Mac's UpNextModel.
//
// Calendar access is the Settings card "Know your meetings". Inkwell never asks on its own: Today
// shows "Show my next meeting", and only a click on it asks. This unpackaged Windows build has no
// calendar (Calendar.cs: NoCalendar, CardState.Unavailable), and Up next says so plainly rather
// than showing nothing or offering to ask.
//
// "in 42 min" is the one thing on Today that changes with the clock. The model holds no timer: the
// view redraws on each whole minute (MinuteSchedule) only while Ticks(onScreen) is true, an event
// is shown and the window is on screen (WindowPresence); hidden, covered or minimised, nothing
// ticks (architecture rule 9). Moving on to the following event when this one starts is one wake
// at that moment, through the injected IWakeScheduler, never a timer that polls.
namespace Inkwell.Core.Screens;

/// <summary>One-shot wakes on the UI thread: the view's implementation uses a DispatcherQueueTimer, the tests a fake.</summary>
public interface IWakeScheduler
{
    /// <summary>Runs <paramref name="wake"/> once on the UI thread after <paramref name="delay"/>; disposing the result cancels it.</summary>
    IDisposable After(TimeSpan delay, Action wake);
}

public static class MeetingApp
{
    private static readonly (string App, string[] Needles)[] Apps =
    [
        ("Zoom", ["zoom.us/", "zoomgov.com/"]),
        ("Teams", ["teams.microsoft.com/", "teams.live.com/"]),
        ("Meet", ["meet.google.com/"]),
        ("Webex", ["webex.com/"]),
        ("FaceTime", ["facetime.apple.com/"]),
        ("Slack", ["slack.com/huddle", "app.slack.com/huddle"]),
    ];

    /// <summary>The meeting app a calendar event's link, place or notes name.</summary>
    public static string? Name(Uri? url, string? location, string? notes)
    {
        var haystack = string.Join(' ', new[] { url?.AbsoluteUri, location, notes }.OfType<string>())
            .ToLowerInvariant();
        foreach (var (app, needles) in Apps)
        {
            if (needles.Any(needle => haystack.Contains(needle, StringComparison.Ordinal)))
            {
                return app;
            }
        }
        return null;
    }

    /// <summary>"in 42 min", "in 1 h 5 min", "now" for an event starting at <paramref name="start"/>.</summary>
    public static string StartsIn(DateTimeOffset start, DateTimeOffset now)
    {
        var minutes = (int)Math.Ceiling((start - now).TotalSeconds / 60);
        if (minutes <= 0)
        {
            return "now";
        }
        if (minutes < 60)
        {
            return $"in {minutes} min";
        }
        var (h, m) = (minutes / 60, minutes % 60);
        return m == 0 ? $"in {h} h" : $"in {h} h {m} min";
    }
}

/// <summary>What Today's Up next shows. UI thread only.</summary>
public sealed class UpNextModel : ObservableModel
{
    /// <summary>How far ahead Up next looks.</summary>
    public static readonly TimeSpan Horizon = TimeSpan.FromHours(12);

    private readonly ICalendarAccess calendarAccess;
    private readonly IUpcomingEvents events;
    private readonly IWakeScheduler scheduler;
    private readonly Func<DateTimeOffset> now;
    private IDisposable? rollover;

    public UpNextModel(ICalendarAccess access, IUpcomingEvents events, IWakeScheduler scheduler, Func<DateTimeOffset>? now = null)
    {
        ArgumentNullException.ThrowIfNull(access);
        ArgumentNullException.ThrowIfNull(events);
        ArgumentNullException.ThrowIfNull(scheduler);
        calendarAccess = access;
        this.events = events;
        this.scheduler = scheduler;
        this.now = now ?? (() => DateTimeOffset.Now);
        Access = access.State();
    }

    public CardState Access { get; private set; }

    public UpcomingEvent? Event { get; private set; }

    /// <summary>Reads the calendars again (the view calls it when Today appears and when the app comes back to the front).</summary>
    public void Refresh()
    {
        Access = calendarAccess.State();
        Event = Access == CardState.Allowed ? events.NextEvent(now(), Horizon) : null;
        rollover?.Dispose();
        rollover = null;
        if (Event is { } shown)
        {
            var wait = (shown.Start - now()) > TimeSpan.Zero ? shown.Start - now() : TimeSpan.Zero;
            rollover = scheduler.After(wait + TimeSpan.FromSeconds(1), Refresh);
        }
        Changed();
    }

    /// <summary>Asks for calendar access (the user clicked), then reads: the prompt the first time, else the settings page.</summary>
    public void Connect() => calendarAccess.Request(Refresh);

    /// <summary>Whether "in N min" redraws on the minute now: an event is shown and the window is on screen.</summary>
    public bool Ticks(bool onScreen) => onScreen && Event is not null;

    /// <summary>What Up next says when it shows no event; null while an event is shown.</summary>
    public string? Note => Event is not null ? null : Access switch
    {
        CardState.Allowed => "Nothing else on your calendar today.",
        CardState.NotAsked or CardState.Checking => "See your next meeting here.",
        CardState.Off or CardState.Unknown => "Calendar access is off, so Inkwell can't show your next meeting.",
        // Windows' unpackaged build: no calendar to ask for.
        _ => "Your calendar isn't available in this version of Inkwell, so it can't show your next meeting.",
    };

    /// <summary>The button that asks (Connect): "Show my next meeting" before asking, "Open Calendar settings" when refused, null otherwise.</summary>
    public string? ConnectTitle => Event is not null ? null : Access switch
    {
        CardState.NotAsked or CardState.Checking => "Show my next meeting",
        CardState.Off or CardState.Unknown => "Open Calendar settings",
        _ => null,
    };

    /// <summary>The event's line under its title: "15:00 · Zoom · in 42 min", at <paramref name="at"/> (the view's minute).</summary>
    public string? MetaLine(DateTimeOffset at, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(calendar);
        if (Event is not { } shown)
        {
            return null;
        }
        var parts = new List<string> { LibraryFormat.Time(shown.Start, calendar) };
        if (shown.App is { } app)
        {
            parts.Add(app);
        }
        parts.Add(MeetingApp.StartsIn(shown.Start, at));
        return string.Join(" · ", parts);
    }

    /// <summary>What happens when it starts: "Records when Zoom opens the microphone".</summary>
    public string? RecordsWhen => Event is not { } shown ? null
        : shown.App is { } app ? $"Records when {app} opens the microphone" : "Records when the call opens the microphone";
}

/// <summary>The minute redraw's times: now, then each whole minute while running; only now while paused (a paused view draws once and then stays still).</summary>
public static class MinuteSchedule
{
    public static IEnumerable<DateTimeOffset> Entries(DateTimeOffset start, bool paused)
    {
        yield return start;
        if (paused)
        {
            yield break;
        }
        var next = start;
        while (true)
        {
            next = NextMinute(next);
            yield return next;
        }
    }

    /// <summary>The next whole minute after <paramref name="date"/>.</summary>
    public static DateTimeOffset NextMinute(DateTimeOffset date)
    {
        var ticks = (date.UtcTicks / TimeSpan.TicksPerMinute + 1) * TimeSpan.TicksPerMinute;
        return new DateTimeOffset(ticks, TimeSpan.Zero).ToOffset(date.Offset);
    }
}

/// <summary>Whether the main window is on screen: open, not minimised, and not fully covered. What redraws on a clock reads it and stops while it is false.</summary>
public sealed class WindowPresence : ObservableModel
{
    public bool OnScreen { get; private set; }

    /// <summary>On screen: visible, not minimised, and some of it not covered.</summary>
    public static bool IsOnScreen(bool visible, bool minimized, bool occlusionVisible) =>
        visible && !minimized && occlusionVisible;

    /// <summary>The window as it is now (the view calls it on every change of these); no window is not on screen.</summary>
    public void Update(bool visible, bool minimized, bool occlusionVisible)
    {
        var onScreen = IsOnScreen(visible, minimized, occlusionVisible);
        if (onScreen != OnScreen)
        {
            OnScreen = onScreen;
            Changed();
        }
    }
}
