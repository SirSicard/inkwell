// The one order records are shown in: newest first by when they started, then by id. The core
// lists them that way; the shell keeps it when pages merge or a record changes, and never sorts
// by anything else (the earlier app's "latest record" was the latest written, not the latest
// held). As the Mac's RecordOrder.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public static class RecordOrder
{
    /// <summary>Whether <paramref name="a"/> comes before <paramref name="b"/>: it started later, or at the same moment with the larger id.</summary>
    public static bool Precedes(RecordRow a, RecordRow b)
    {
        ArgumentNullException.ThrowIfNull(a);
        ArgumentNullException.ThrowIfNull(b);
        return Compare(a, b) < 0;
    }

    /// <summary><paramref name="rows"/> newest first, each record once (a later copy replaces an earlier one).</summary>
    public static IReadOnlyList<RecordRow> NewestFirst(IEnumerable<RecordRow> rows)
    {
        ArgumentNullException.ThrowIfNull(rows);
        var byId = new Dictionary<string, RecordRow>(StringComparer.Ordinal);
        foreach (var row in rows)
        {
            byId[row.Record] = row;
        }
        var sorted = byId.Values.ToList();
        sorted.Sort(Compare);
        return sorted;
    }

    /// <summary>The meeting that started last among those that have ended: a meeting still being recorded is not "the last meeting".</summary>
    public static RecordRow? LatestFinishedMeeting(IEnumerable<RecordRow> rows)
    {
        ArgumentNullException.ThrowIfNull(rows);
        var finished = NewestFirst(rows.Where(r => r.Kind == RecordKind.Meeting && r.EndedAtUnixMs is not null));
        return finished.Count > 0 ? finished[0] : null;
    }

    /// <summary>Newest first: a negative number when <paramref name="a"/> is shown before <paramref name="b"/>.</summary>
    private static int Compare(RecordRow a, RecordRow b)
    {
        if (a.StartedAtUnixMs != b.StartedAtUnixMs)
        {
            return b.StartedAtUnixMs.CompareTo(a.StartedAtUnixMs);
        }
        return string.CompareOrdinal(b.Record, a.Record);
    }
}
