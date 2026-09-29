// The calendar, as the screens see it: the "Know your meetings" permission card, Today's Up next,
// and a meeting's title. The Mac reads EventKit. Windows' calendar API (AppointmentManager) is
// only open to a packaged app with the appointments capability, and this build is unpackaged, so
// the app passes NoCalendar until packaging (S3.6) brings one: every screen then says the calendar
// is not available, never that it is empty or refused.
namespace Inkwell.Core.Screens;

/// <summary>A permission card's state.</summary>
public enum CardState
{
    /// <summary>Not known yet: the first check has not answered.</summary>
    Checking,
    /// <summary>Granted.</summary>
    Allowed,
    /// <summary>Refused or revoked: the card is red and says what is lost.</summary>
    Off,
    /// <summary>Never asked: Allow shows the system prompt.</summary>
    NotAsked,
    /// <summary>The system gives no answer it can be sure of.</summary>
    Unknown,
    /// <summary>This build cannot reach it at all (Windows: the calendar of an unpackaged app).</summary>
    Unavailable,
}

public static class CardStates
{
    /// <summary>Whether the card shows the alert colour.</summary>
    public static bool IsAlert(this CardState state) => state == CardState.Off;
}

/// <summary>Calendar access: its state now, and asking for it.</summary>
public interface ICalendarAccess
{
    /// <summary>Now, without prompting.</summary>
    CardState State();

    /// <summary>Shows the prompt if never asked, else opens the settings page. <paramref name="done"/> runs on the UI thread.</summary>
    void Request(Action done);
}

/// <summary>A calendar event that is coming up.</summary>
/// <param name="App">The meeting app its link, place or notes name (Zoom, Teams, Meet, Webex), if any.</param>
public sealed record UpcomingEvent(string Title, DateTimeOffset Start, DateTimeOffset End, string? App);

/// <summary>Where Today reads the next event.</summary>
public interface IUpcomingEvents
{
    /// <summary>The next timed event starting after <paramref name="now"/>, within <paramref name="horizon"/>; null without access.</summary>
    UpcomingEvent? NextEvent(DateTimeOffset now, TimeSpan horizon);
}

/// <summary>A meeting's title from the calendar.</summary>
public interface ICallTitles
{
    /// <summary>The title of the calendar event going on at <paramref name="now"/>, if there is one and the calendar may be read. Never prompts.</summary>
    string? TitleNow(DateTimeOffset now);
}

/// <summary>This build's calendar: none (see the file header).</summary>
public sealed class NoCalendar : ICalendarAccess, IUpcomingEvents, ICallTitles
{
    public static NoCalendar Instance { get; } = new();

    public CardState State() => CardState.Unavailable;

    public void Request(Action done)
    {
        ArgumentNullException.ThrowIfNull(done);
        done();
    }

    public UpcomingEvent? NextEvent(DateTimeOffset now, TimeSpan horizon) => null;

    public string? TitleNow(DateTimeOffset now) => null;
}
