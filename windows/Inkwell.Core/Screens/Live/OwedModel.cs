// Owed: what was promised in meetings and is still open, grouped, with what is overdue, as the
// Mac's OwedModel.
//
// The core lists the open commitments (not done, and not merged into another: a promise said twice
// is one row with "Said twice"), soonest due first. Grouping: by the person a promise is owed to
// ("To Dana") when the meeting said; else, a commitment with a named owner under that person; the
// rest under the meeting they were made in.
//
// "Looks done" suggestions show above the list: a later meeting in which the user said the work
// was already done (the core matches what was said to the open promises). "Mark done" closes the
// promise; "Not yet" dismisses the suggestion, and the promise stays.
using System.Collections.Immutable;
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>When something is due, as the screen shows it.</summary>
public abstract record DueLabel
{
    private DueLabel() { }

    public sealed record Undated : DueLabel;

    /// <summary>Before today: days late.</summary>
    public sealed record Overdue(int Days) : DueLabel;

    public sealed record Today : DueLabel;

    public sealed record Tomorrow : DueLabel;

    /// <summary>Within the next six days: the weekday. <paramref name="Date"/> is in the model's time zone.</summary>
    public sealed record ThisWeek(DateTimeOffset Date) : DueLabel;

    /// <summary>Later. <paramref name="Date"/> is in the model's time zone.</summary>
    public sealed record Later(DateTimeOffset Date) : DueLabel;

    public bool IsOverdue => this is Overdue;

    /// <summary>Due today or within the week, or late.</summary>
    public bool IsThisWeek => this is Today or Tomorrow or ThisWeek or Overdue;

    public string Text => this switch
    {
        Overdue { Days: 1 } => "1 day overdue",
        Overdue o => string.Create(CultureInfo.InvariantCulture, $"{o.Days} days overdue"),
        Today => "Due today",
        Tomorrow => "Due tomorrow",
        ThisWeek w => "Due " + w.Date.ToString("ddd", CultureInfo.CurrentCulture),
        Later l => "Due " + OwedDates.DayMonth(l.Date),
        _ => "No date",
    };

    public static DueLabel Of(DateTimeOffset? due, DateTimeOffset now, TimeZoneInfo zone)
    {
        ArgumentNullException.ThrowIfNull(zone);
        if (due is not DateTimeOffset at)
        {
            return new Undated();
        }
        var local = TimeZoneInfo.ConvertTime(at, zone);
        var days = (local.Date - TimeZoneInfo.ConvertTime(now, zone).Date).Days;
        return days switch
        {
            < 0 => new Overdue(-days),
            // Due dates are days ("by Friday"): anything due today is due today, not late.
            0 => new Today(),
            1 => new Tomorrow(),
            <= 6 => new ThisWeek(local),
            _ => new Later(local),
        };
    }
}

/// <summary>The dates Owed shows, in the user's culture (the Mac's weekday, day and abbreviated month).</summary>
public static class OwedDates
{
    /// <summary>"Tue, Sep 29".</summary>
    public static string WeekdayDayMonth(DateTimeOffset date) => date.ToString("ddd, MMM d", CultureInfo.CurrentCulture);

    /// <summary>"Oct 3".</summary>
    public static string DayMonth(DateTimeOffset date) => date.ToString("MMM d", CultureInfo.CurrentCulture);

    internal static DateTimeOffset FromUnixMs(long ms, TimeZoneInfo zone) =>
        TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeMilliseconds(ms), zone);
}

/// <summary>One promise as the screen lists it.</summary>
/// <param name="Merged">Times it was said again and merged into this one.</param>
/// <param name="Record">The record it was said in.</param>
/// <param name="SaidAtMs">Where in that record.</param>
/// <param name="Meeting">The meeting's title, for a row listed under a person.</param>
public sealed record OwedRow(string Id, string Text, DueLabel Due, int Merged, string Record, long? SaidAtMs, string? Meeting)
{
    /// <summary>"Said twice · merged", "Said 3 times · merged", or null.</summary>
    public string? MergedText => Merged switch
    {
        0 => null,
        1 => "Said twice · merged",
        _ => string.Create(CultureInfo.InvariantCulture, $"Said {Merged + 1} times · merged"),
    };

    /// <summary>Where it was said: "Partner call at 38:52" (the meeting only under a person), or null.</summary>
    public string? SaidLine => SaidAtMs is long at
        ? string.Join(" ", new[] { Meeting, $"at {LiveClock.Of(at)}" }.OfType<string>())
        : null;
}

/// <summary>A heading and its promises.</summary>
public sealed record OwedGroup(string Id, string Title, string? Subtitle, IReadOnlyList<OwedRow> Rows)
{
    public bool Equals(OwedGroup? other) =>
        other is not null && Id == other.Id && Title == other.Title && Subtitle == other.Subtitle && Rows.SequenceEqual(other.Rows);

    public override int GetHashCode() => HashCode.Combine(Id, Title, Subtitle, Rows.Count);
}

/// <summary>A later meeting suggests a promise was kept.</summary>
/// <param name="Commitment">The commitment it would close.</param>
/// <param name="Text">What was said, in the user's words.</param>
/// <param name="Source">Where: the meeting and the time into it.</param>
public sealed record LooksDone(string Id, string Commitment, string Text, string Source)
{
    /// <summary>The card's line (shown as plain text: what was said is never parsed).</summary>
    public string Headline => "Looks done: " + Text;
}

public sealed class OwedModel : ObservableModel
{
    private readonly Action<CoreCommand> send;
    private readonly TimeZoneInfo zone;

    /// <param name="zone">The time zone days are counted in (tests pass UTC).</param>
    public OwedModel(Action<CoreCommand> send, TimeZoneInfo? zone = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.zone = zone ?? TimeZoneInfo.Local;
    }

    public ImmutableList<OwedItem> Items { get; private set; } = [];
    /// <summary>The core has listed them at least once.</summary>
    public bool Loaded { get; private set; }
    public ImmutableList<LooksDone> Suggestions { get; private set; } = [];
    /// <summary>
    /// The last answer the core refused, in words, until the next answer: the promise is back in
    /// the list as the core has it.
    /// </summary>
    public string? Failure { get; private set; }
    /// <summary>
    /// The list could not be read, in words, until it is. Not on the Mac (whose screen then keeps
    /// its spinner): a failed load reads "Couldn't ...", never loading or empty.
    /// </summary>
    public string? LoadFailure { get; private set; }

    /// <summary>Still waiting for the first list.</summary>
    public bool Loading => !Loaded && LoadFailure is null;

    public void Load() => send(new CoreCommand.CommitmentsList());

    /// <summary>The Undo toast's button.</summary>
    public const string UndoTitle = "Undo";

    /// <summary>How long the Undo toast stays.</summary>
    public static readonly TimeSpan UndoShown = TimeSpan.FromSeconds(6);

    /// <summary>The promise marked done last, while its Undo toast shows; null when none does.</summary>
    public OwedItem? JustDone { get; private set; }

    /// <summary>The Undo toast's line: "Marked done: Send the revised plan".</summary>
    public string? JustDoneLine => JustDone is { } item ? $"Marked done: {item.Text}" : null;

    /// <summary>Marks a promise done: it leaves the list at once, and the core's list replaces it.</summary>
    public void MarkDone(string id)
    {
        Failure = null;
        JustDone = Items.Find(i => i.Id == id) ?? JustDone;
        Items = Items.RemoveAll(i => i.Id == id);
        Suggestions = Suggestions.RemoveAll(s => s.Commitment == id);
        send(new CoreCommand.CommitmentSetDone(id, true));
        Changed();
    }

    /// <summary>The toast's Undo: the promise marked done last is open again (the core's list brings it back).</summary>
    public void UndoDone()
    {
        if (JustDone is not { } item)
        {
            return;
        }
        Failure = null;
        JustDone = null;
        send(new CoreCommand.CommitmentSetDone(item.Id, false));
        Changed();
    }

    /// <summary>The toast went (its time ran out, or the screen went): nothing to undo any more.</summary>
    public void UndoExpired()
    {
        if (JustDone is not null)
        {
            JustDone = null;
            Changed();
        }
    }

    /// <summary>The user says a suggestion is wrong: it goes, the promise stays.</summary>
    public void NotYet(LooksDone suggestion)
    {
        ArgumentNullException.ThrowIfNull(suggestion);
        Failure = null;
        Suggestions = Suggestions.RemoveAll(s => s.Id == suggestion.Id);
        send(new CoreCommand.CommitmentNotYet(suggestion.Commitment));
        Changed();
    }

    /// <summary>The suggestions in the core's list: each open promise a later meeting says looks done.</summary>
    public static ImmutableList<LooksDone> SuggestionsOf(IEnumerable<OwedItem> items, TimeZoneInfo zone)
    {
        ArgumentNullException.ThrowIfNull(items);
        ArgumentNullException.ThrowIfNull(zone);
        var suggestions = ImmutableList.CreateBuilder<LooksDone>();
        foreach (var item in items)
        {
            if (item.LooksDone is not DoneEvidence evidence)
            {
                continue;
            }
            var title = NonEmpty(evidence.RecordTitle?.Trim());
            var meeting = title
                ?? (evidence.RecordStartedAtUnixMs is long started
                    ? "a meeting on " + OwedDates.WeekdayDayMonth(OwedDates.FromUnixMs(started, zone))
                    : "a later meeting");
            var text = NonEmpty(evidence.Text?.Trim()) is string said
                ? $"in {meeting} you said “{said}”"
                : $"{meeting} suggests “{item.Text}” is done";
            suggestions.Add(new LooksDone(
                $"looks-done:{item.Id}", item.Id, text, $"{title ?? "That meeting"} ▸ {LiveClock.Of(evidence.Span.StartMs)}"));
        }
        return suggestions.ToImmutable();
    }

    /// <summary>"5 open · 2 due this week · 1 overdue".</summary>
    public string Summary(DateTimeOffset now)
    {
        var dues = Items.Select(i => Due(i, now)).ToList();
        var parts = new List<string> { string.Create(CultureInfo.InvariantCulture, $"{Items.Count} open") };
        var week = dues.Count(d => d.IsThisWeek && !d.IsOverdue);
        var late = dues.Count(d => d.IsOverdue);
        if (week > 0)
        {
            parts.Add(string.Create(CultureInfo.InvariantCulture, $"{week} due this week"));
        }
        if (late > 0)
        {
            parts.Add(string.Create(CultureInfo.InvariantCulture, $"{late} overdue"));
        }
        return string.Join(" · ", parts);
    }

    /// <summary>How many promises are overdue now (the navigation's count beside Owed).</summary>
    public int OverdueCount(DateTimeOffset now) => Items.Count(i => Due(i, now).IsOverdue);

    public DueLabel Due(OwedItem item, DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(item);
        return DueLabel.Of(item.DueAtUnixMs is long due ? DateTimeOffset.FromUnixTimeMilliseconds(due) : null, now, zone);
    }

    /// <summary>The groups, in the order of their soonest promise (the core's order).</summary>
    public IReadOnlyList<OwedGroup> Groups(DateTimeOffset now)
    {
        var order = new List<string>();
        var titles = new Dictionary<string, (string Title, string? Subtitle)>();
        var rows = new Dictionary<string, List<OwedRow>>();
        foreach (var item in Items)
        {
            var owner = item.Owner?.Trim();
            var recipient = item.Recipient?.Trim();
            // The summary names people as the transcript does: "You", "Them", a name. The user's
            // own promises are grouped by meeting, like those with no owner; "Them" alone is no
            // heading.
            var ownedBySomeoneElse = !string.IsNullOrEmpty(owner) && !owner.Equals("you", StringComparison.OrdinalIgnoreCase);
            string key;
            if (!string.IsNullOrEmpty(recipient))
            {
                key = "to:" + recipient.ToLowerInvariant();
                titles.TryAdd(key, (recipient.Equals("you", StringComparison.OrdinalIgnoreCase) ? "Owed to you" : "To " + recipient, null));
            }
            else if (ownedBySomeoneElse && owner is not null)
            {
                key = "owner:" + owner.ToLowerInvariant();
                titles.TryAdd(key, (owner.Equals("them", StringComparison.OrdinalIgnoreCase) ? "Owed by the others" : owner, null));
            }
            else
            {
                key = "record:" + item.Record;
                // An untitled meeting is already named by its day: no second date beside it.
                var titled = !string.IsNullOrEmpty(item.RecordTitle?.Trim());
                titles.TryAdd(key, (MeetingTitle(item), titled ? MeetingDay(item, now) : null));
            }
            if (!rows.TryGetValue(key, out var list))
            {
                order.Add(key);
                rows[key] = list = [];
            }
            list.Add(new OwedRow(
                item.Id, item.Text, Due(item, now), (int)item.Merged, item.Record, item.SaidAtMs,
                key.StartsWith("record:", StringComparison.Ordinal) ? null : MeetingTitle(item)));
        }
        return order.Select(key => new OwedGroup(key, titles[key].Title, titles[key].Subtitle, rows[key])).ToList();
    }

    private DateTimeOffset Started(OwedItem item) => OwedDates.FromUnixMs(item.RecordStartedAtUnixMs, zone);

    private string MeetingTitle(OwedItem item) =>
        NonEmpty(item.RecordTitle?.Trim()) ?? "A meeting on " + OwedDates.WeekdayDayMonth(Started(item));

    private string MeetingDay(OwedItem item, DateTimeOffset now)
    {
        var day = Started(item).Date;
        var today = TimeZoneInfo.ConvertTime(now, zone).Date;
        if (day == today)
        {
            return "today";
        }
        if (day == today.AddDays(-1))
        {
            return "yesterday";
        }
        return OwedDates.WeekdayDayMonth(Started(item));
    }

    /// <summary>Whether this model shows the failure (the rest the aggregator logs).</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "commitment.set_done" or "commitment.not_yet" or "commitments.list";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case CommitmentsListed listed:
                Items = [.. listed.Items];
                Suggestions = SuggestionsOf(listed.Items, zone);
                Loaded = true;
                LoadFailure = null;
                break;
            case CommitmentUpdated or MeetingCommitments or MeetingLooksDone or Events.LibrarySwept or RecordDeleted:
                // A promise changed, a meeting filed new ones or found some done, or old ones (or a
                // record the user deleted) went: list again.
                Load();
                return;
            case CommandFailed failed when failed.Command is "commitment.set_done" or "commitment.not_yet":
                // Put it back as the core has it, and say why it came back.
                Failure = failed.Command == "commitment.set_done"
                    ? $"Couldn't mark it done: {failed.Message}"
                    : $"Couldn't keep it open: {failed.Message}";
                Load();
                break;
            case CommandFailed failed when failed.Command == "commitments.list":
                LoadFailure = $"Couldn't load what you owe: {failed.Message}";
                break;
            default:
                return;
        }
        Changed();
    }

    private static string? NonEmpty(string? s) => string.IsNullOrEmpty(s) ? null : s;
}
