// One record as the Record screen reads it, built from the core's `library.record` answer: the
// ledger (who said what, when), the notes-first merged view, the rendered summary with each
// decision's and action's cited line, what is owed and the audio's chunks. As the Mac's
// RecordDocument, plus the words the Record screen composes from it (tab titles, the transcript
// Copy puts on the clipboard), so the XAML stays thin. Built once per answer; immutable.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>Who said a line: the mic is the user; the far end is a name the user gave, a numbered speaker the diarizer found, or the far end as a whole when labels were not kept.</summary>
public sealed record Speaker
{
    private Speaker(bool isYou, string label)
    {
        IsYou = isYou;
        Label = label;
    }

    /// <summary>The mic: the user.</summary>
    public static Speaker You { get; } = new(true, "You");

    /// <summary>The far end, as named or numbered.</summary>
    public static Speaker Them(string name) => new(false, name);

    public bool IsYou { get; }

    public string Label { get; }
}

/// <summary>One line of the ledger transcript; <paramref name="Id"/> is its index in the transcript.</summary>
public sealed record LedgerLine(int Id, Speaker Speaker, long StartMs, long EndMs, string Text);

/// <summary>What an entry of the notes-first merged view is.</summary>
public enum MergedKind
{
    /// <summary>A note the user typed.</summary>
    Note,
    /// <summary>What was said when the note was written (its speaker is set).</summary>
    Said,
    /// <summary>A commitment made around then.</summary>
    Owed,
}

/// <summary>One entry of the notes-first merged view; <paramref name="AtMs"/> is where its chip points on the record's timeline.</summary>
public sealed record MergedEntry(string Id, MergedKind Kind, string Text, long AtMs, Speaker? Speaker = null);

/// <summary>A commitment as the Record screen lists it, with the line it was said in.</summary>
/// <param name="AtMs">Where it was said, when the core knows.</param>
/// <param name="CitedLine">The transcript line holding that moment: its source, shown beside it so a mismatch is visible at a glance.</param>
public sealed record OwedEntry(string Id, string Text, string? Owner, string? Due, bool Done, long? AtMs, LedgerLine? CitedLine)
{
    /// <summary>The line under it: <c>You · Friday</c>.</summary>
    public string MetaLine => string.Join(" · ", new[] { Owner, Due }.OfType<string>());

    /// <summary>What its check button says to a screen reader.</summary>
    public string ToggleLabel => Done ? $"Mark not done: {Text}" : $"Mark done: {Text}";
}

/// <summary>A playable chunk of a record's audio, on its timeline.</summary>
public sealed record TimelineChunk(Channel Channel, string Path, long StartMs, long Frames, int SampleRate, int Channels, long DataOffset)
{
    public long DurationMs => SampleRate > 0 ? Frames * 1000 / SampleRate : 0;

    public long EndMs => StartMs + DurationMs;
}

/// <summary>A summary's decision or action with the transcript line it cites (S2.8: the core keeps each item's span with the summary).</summary>
/// <param name="CitedLine">The line at that moment on that side: the item's source, shown beside it.</param>
public sealed record SummaryCitation(int Id, SummaryItemKind Kind, string Text, long AtMs, LedgerLine? CitedLine)
{
    /// <summary>The cited line as the screen quotes it: <c>You: “…”</c>.</summary>
    public string? Quote => CitedLine is { } line ? RecordDocument.QuoteOf(line) : null;
}

/// <summary>What the player cannot vouch for about a record's audio.</summary>
/// <param name="TimelineEstimated">The chunks were placed from the earliest one, the meeting's start not being written: the two sides may be out of step.</param>
/// <param name="LeftOut">Chunk files the core left out of the player (unreadable, or of unknown format or place).</param>
public sealed record PlaybackCaveats(bool TimelineEstimated, int LeftOut)
{
    /// <summary>The words the player bar shows, one per line; <paramref name="waveformPartial"/> when a chunk could not be read for the waveform.</summary>
    public IReadOnlyList<string> Messages(bool waveformPartial)
    {
        var lines = new List<string>();
        if (TimelineEstimated)
        {
            lines.Add("Timing estimated: the two sides may be out of step.");
        }
        if (LeftOut > 0 || waveformPartial)
        {
            lines.Add("Part of this recording can't be played.");
        }
        return lines;
    }
}

/// <summary>The Record screen's tabs.</summary>
public enum RecordTab
{
    Notes,
    Summary,
    Owed,
}

public sealed class RecordDocument
{
    /// <summary>Lines from this long before a note count as "what was being said".</summary>
    private const long Lead = 5_000;

    public RecordRow Record { get; }

    public IReadOnlyList<LedgerLine> Ledger { get; }

    public IReadOnlyList<MergedEntry> Merged { get; }

    public SummaryDocument? Summary { get; }

    public long? SummaryWrittenAt { get; }

    /// <summary>The summary's decisions and actions, each with its cited line, in the summary's order.</summary>
    public IReadOnlyList<SummaryCitation> SummaryItems { get; }

    /// <summary>The record's own commitments that stand (not folded into another), in the order said.</summary>
    public IReadOnlyList<OwedEntry> Owed { get; }

    /// <summary>The people on the far end, as named or numbered.</summary>
    public IReadOnlyList<string> People { get; }

    public IReadOnlyList<TimelineChunk> Chunks { get; }

    /// <summary>
    /// What the player cannot vouch for: the screen shows each (beside the ledger's status and in
    /// the player bar), so an estimated or partial recording never plays with a precise one's
    /// confidence.
    /// </summary>
    public PlaybackCaveats PlaybackCaveats { get; }

    /// <summary>The transcript is the final pass's (revision 2 or later), not the live one.</summary>
    public bool IsFinal { get; }

    /// <summary>How far the chips may go: the audio's end, else the last line's.</summary>
    public long DurationMs =>
        Math.Max(Chunks.Count > 0 ? Chunks.Max(c => c.EndMs) : 0, Ledger.Count > 0 ? Ledger.Max(l => l.EndMs) : 0);

    public RecordDocument(LibraryRecord answer)
    {
        ArgumentNullException.ThrowIfNull(answer);
        Record = answer.Record;
        IsFinal = answer.Record.Revision >= 2;
        var names = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var named in answer.Speakers)
        {
            names.TryAdd(named.Speaker, named.Name);
        }
        // Unnamed diarized speakers are numbered in the order they first speak.
        var numbered = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (var segment in answer.Segments)
        {
            if (segment.Channel == Channel.Far && segment.Speaker is string label && !names.ContainsKey(label))
            {
                numbered.TryAdd(label, numbered.Count + 1);
            }
        }
        Speaker SpeakerOf(RecordSegment segment)
        {
            if (segment.Channel != Channel.Far)
            {
                return Speaker.You;
            }
            if (segment.Speaker is not string label)
            {
                return Speaker.Them("Them");
            }
            if (names.TryGetValue(label, out var name))
            {
                return Speaker.Them(name);
            }
            return Speaker.Them(string.Create(CultureInfo.InvariantCulture, $"Speaker {numbered.GetValueOrDefault(label, 1)}"));
        }
        var ledger = answer.Segments
            .Select((segment, index) => new LedgerLine(index, SpeakerOf(segment), segment.StartMs, segment.EndMs, segment.Text.Trim()))
            .ToList();
        Ledger = ledger;

        var people = new List<string>();
        foreach (var line in ledger)
        {
            if (!line.Speaker.IsYou && !people.Contains(line.Speaker.Label))
            {
                people.Add(line.Speaker.Label);
            }
        }
        People = people;

        Summary = answer.Summary is { } summary ? new SummaryDocument(summary.Text) : null;
        SummaryWrittenAt = answer.Summary?.CreatedAtUnixMs;
        SummaryItems = (answer.Summary?.Items ?? [])
            .Select((item, index) => new SummaryCitation(
                index, item.Kind, item.Text, item.Span.StartMs, LineAt(item.Span.StartMs, ledger, item.Span.Channel)))
            .ToList();

        Owed = answer.Commitments
            .Where(c => c.MergedInto is null)
            .Select(c =>
            {
                long? at = c.Provenance.Count > 0 ? c.Provenance.Min(p => p.StartMs) : null;
                var cited = at is long ms ? LineAt(ms, ledger, c.Provenance[0].Channel) : null;
                return new OwedEntry(c.Commitment, c.Text, c.Owner, c.Due, c.Done, at, cited);
            })
            .ToList();

        var notes = answer.Notes.ToList();
        notes.Sort((a, b) => a.AtMs != b.AtMs ? a.AtMs.CompareTo(b.AtMs) : string.CompareOrdinal(a.Note, b.Note));
        Merged = Merge(notes, ledger, Owed);

        Chunks = (answer.Audio?.Chunks ?? [])
            .Select(c => new TimelineChunk(c.Channel, c.Path, c.StartMs, c.Frames, (int)c.SampleRate, (int)c.Channels, c.DataOffset))
            .ToList();
        PlaybackCaveats = new PlaybackCaveats(
            answer.Audio?.Timeline == AudioTimeline.Estimated, (int)(answer.Audio?.LeftOut ?? 0));
    }

    /// <summary>
    /// The ledger's status: blotted (the final pass's transcript) or live, and whether the audio's
    /// timing is estimated. <paramref name="blottedAt"/> is the summary's time, formatted.
    /// </summary>
    public string LedgerStatus(string? blottedAt)
    {
        var parts = IsFinal
            ? new List<string> { blottedAt is null ? "Blotted" : $"Blotted {blottedAt}", "final" }
            : ["Live transcript", "not blotted"];
        if (PlaybackCaveats.TimelineEstimated)
        {
            parts.Add("timing estimated");
        }
        return string.Join(" · ", parts);
    }

    /// <summary>The ledger's status with the summary's time read on <paramref name="calendar"/>.</summary>
    public string LedgerStatus(LibraryCalendar calendar) =>
        LedgerStatus(SummaryWrittenAt is long at ? LibraryFormat.Time(LibraryFormat.Date(at), calendar) : null);

    /// <summary>The ledger line to highlight while the playhead is at <paramref name="ms"/>: the last line that has started.</summary>
    public LedgerLine? LineAtPlayhead(long ms) => Ledger.LastOrDefault(l => l.StartMs <= ms);

    /// <summary>The line holding <paramref name="ms"/> on <paramref name="channel"/> (or either side), else the nearest one before it.</summary>
    public static LedgerLine? LineAt(long ms, IReadOnlyList<LedgerLine> ledger, Channel? channel)
    {
        ArgumentNullException.ThrowIfNull(ledger);
        var side = ledger.Where(line => channel is not Channel c || (c == Channel.Mic) == line.Speaker.IsYou).ToList();
        // Where one line ends as the next starts, the one starting there.
        return side.LastOrDefault(l => l.StartMs <= ms && ms <= l.EndMs)
            ?? side.LastOrDefault(l => l.StartMs <= ms)
            ?? side.FirstOrDefault();
    }

    /// <summary>
    /// Notes first: each note, then what was said as it was written (the first lines from a few
    /// seconds before it), then what was promised before the next note. Without notes, the
    /// record's commitments lead, each with its moment.
    /// </summary>
    public static IReadOnlyList<MergedEntry> Merge(IReadOnlyList<RecordNote> notes, IReadOnlyList<LedgerLine> ledger, IReadOnlyList<OwedEntry> owed)
    {
        ArgumentNullException.ThrowIfNull(notes);
        ArgumentNullException.ThrowIfNull(ledger);
        ArgumentNullException.ThrowIfNull(owed);
        var entries = new List<MergedEntry>();
        var placed = new HashSet<string>(StringComparer.Ordinal);
        for (var i = 0; i < notes.Count; i++)
        {
            var note = notes[i];
            var next = i + 1 < notes.Count ? notes[i + 1].AtMs : long.MaxValue;
            entries.Add(new MergedEntry($"note-{note.Note}", MergedKind.Note, note.Text, note.AtMs));
            foreach (var line in ledger.Where(l => l.StartMs >= note.AtMs - Lead && l.StartMs < next).Take(2))
            {
                entries.Add(new MergedEntry(
                    string.Create(CultureInfo.InvariantCulture, $"said-{note.Note}-{line.Id}"), MergedKind.Said, line.Text, line.StartMs, line.Speaker));
            }
            foreach (var item in owed)
            {
                if (placed.Contains(item.Id) || item.AtMs is not long at || at < note.AtMs - Lead || at >= next)
                {
                    continue;
                }
                placed.Add(item.Id);
                entries.Add(new MergedEntry($"owed-{item.Id}", MergedKind.Owed, item.Text, at));
            }
        }
        // Commitments no note sits near (all of them, without notes).
        foreach (var item in owed)
        {
            if (!placed.Contains(item.Id) && item.AtMs is long at)
            {
                entries.Add(new MergedEntry($"owed-{item.Id}", MergedKind.Owed, item.Text, at));
            }
        }
        return entries;
    }

    // What the Record screen composes (the Mac's RecordScreen).

    /// <summary>Whether the merged view holds any of the user's notes.</summary>
    public bool HasNotes => Merged.Any(e => e.Kind == MergedKind.Note);

    /// <summary>The notes column's heading.</summary>
    public string NotesHeading => HasNotes ? "Your notes, filled in" : "Key moments";

    /// <summary>What the notes column says when there are none; null when there are.</summary>
    public string? NoNotesText => HasNotes ? null : Record.Kind == RecordKind.Meeting ? "You typed no notes in this meeting." : "No notes.";

    /// <summary>The people the heading names: you first, in a meeting.</summary>
    public IReadOnlyList<string> HeaderPeople => Record.Kind == RecordKind.Meeting ? ["You", .. People] : People;

    /// <summary>The heading's second line.</summary>
    public string HeaderLine(DateTimeOffset now, LibraryCalendar calendar) => LibraryFormat.HeaderLine(Record, HeaderPeople, now, calendar);

    /// <summary>What is owed and not done yet: the Owed tab's count.</summary>
    public int OwedCount => Owed.Count(o => !o.Done);

    /// <summary>A tab's title: <c>Owed · 2</c> while something is owed.</summary>
    public string TabTitle(RecordTab tab) => tab switch
    {
        RecordTab.Notes => "Notes and transcript",
        RecordTab.Summary => "Summary",
        _ => OwedCount > 0 ? string.Create(CultureInfo.InvariantCulture, $"Owed · {OwedCount}") : "Owed",
    };

    /// <summary>The transcript as Copy transcript puts it on the clipboard: <c>00:12 You: …</c> per line.</summary>
    public string TranscriptText => string.Join("\n", Ledger.Select(l => $"{LibraryFormat.Stamp(l.StartMs)} {l.Speaker.Label}: {l.Text}"));

    /// <summary>What Share sends: the title, then the summary (else the transcript).</summary>
    public string ShareText => $"{LibraryFormat.Title(Record)}\n\n{Summary?.PlainText ?? TranscriptText}";

    /// <summary>The summary's decisions: "Decided, and where".</summary>
    public IReadOnlyList<SummaryCitation> Decisions => SummaryItems.Where(i => i.Kind == SummaryItemKind.Decision).ToList();

    /// <summary>What is owed with the line it was said in: "Where it was said".</summary>
    public IReadOnlyList<OwedEntry> CitedOwed => Owed.Where(o => o.CitedLine is not null).ToList();

    /// <summary>What the Summary tab says without a summary: the AI settings' note when summaries are off, else when one is written.</summary>
    public static string NoSummaryText(string? summaryOffNote) =>
        summaryOffNote ?? "A summary is written after a meeting ends, when a language model is set up.";

    /// <summary>The Summary tab's heading without a summary.</summary>
    public const string NoSummaryTitle = "No summary yet";

    /// <summary>What the Owed tab says when nothing was promised.</summary>
    public const string NothingOwedText = "Nothing was promised in this record.";

    /// <summary>What the ledger says when nothing was transcribed.</summary>
    public const string EmptyLedgerText = "Nothing was transcribed.";

    /// <summary>A line as a cited item quotes it.</summary>
    public static string QuoteOf(LedgerLine line)
    {
        ArgumentNullException.ThrowIfNull(line);
        return $"{line.Speaker.Label}: “{line.Text}”";
    }

    /// <summary>A ledger row as a screen reader reads it.</summary>
    public static string LineLabel(LedgerLine line)
    {
        ArgumentNullException.ThrowIfNull(line);
        return $"{LibraryFormat.Stamp(line.StartMs)}, {line.Speaker.Label}: {line.Text}";
    }

    /// <summary>A timestamp chip as a screen reader reads it.</summary>
    public static string ChipLabel(long ms) => $"Play from {LibraryFormat.Stamp(ms)}";
}
