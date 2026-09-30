// What the Today, Library and Record screens show of the library, built from the core's answers
// to the library's commands (CoreCommand.RecordsList and the rest), as the Mac's LibraryModel. The
// screens ask by calling the refresh methods; answers arrive as events, on the UI thread. What is
// owed and the permissions are the Owed and Settings models': Today reads them there.
//
// Each question goes out with an id naming its slot (the list, a search, the open record, ...)
// and a number, and the answer echoes it as `ref`. Only the answer to a slot's newest question is
// kept: typing a search fast, or clicking through records, never shows an older answer over a
// newer one. A question that fails is a `command.failed` with that id, and the slot then reads as
// "could not load", never as an empty library.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>Where an answer is: asked and not answered, answered, or failed.</summary>
public enum LibraryLoad
{
    Idle,
    Loading,
    Loaded,
    Failed,
}

/// <summary>Whether the far end recorded nothing lately, as far as the counts can tell (Today's needs-you banner).</summary>
public abstract record FarEndCheck
{
    private FarEndCheck() { }

    /// <summary>Not answered yet: no guess either way.</summary>
    public sealed record Unknown : FarEndCheck;

    /// <summary>The counts could not be read: said, never taken as "none".</summary>
    public sealed record Failed : FarEndCheck;

    /// <summary>The newest meetings in a row that kept only the user's voice, and since when.</summary>
    public sealed record Checked(long Meetings, DateTimeOffset? Since) : FarEndCheck;
}

/// <summary>Runs an action once after a delay, on the UI thread; disposing the handle cancels it. The view's implementation is a one-shot DispatcherQueue timer, never a polling one.</summary>
public interface ISearchScheduler
{
    IDisposable After(TimeSpan delay, Action action);
}

/// <summary>A kind filter chip: the kind, its title, and its title in a sentence.</summary>
public sealed record KindChip(RecordKind Kind, string Title, string InSentence);

public sealed class LibraryModel : ObservableModel
{
    /// <summary>Records per page.</summary>
    public const int PageSize = 100;

    /// <summary>The most characters a search asks with.</summary>
    public const int MaxQueryLength = 200;

    /// <summary>How long typing must pause before a search is asked: one question per pause, not one per key.</summary>
    public static readonly TimeSpan SearchDelay = TimeSpan.FromMilliseconds(250);

    /// <summary>Meetings, Dictations, Files: one at a time, or none for everything.</summary>
    public static IReadOnlyList<KindChip> Kinds { get; } =
    [
        new(RecordKind.Meeting, "Meetings", "meetings"),
        new(RecordKind.Dictation, "Dictations", "dictations"),
        new(RecordKind.FileImport, "Files", "files"),
    ];

    /// <summary>What a question is for (the Mac's slot names, which the ids carry).</summary>
    private enum Slot
    {
        List,
        More,
        Search,
        Open,
        TodayMeetings,
        TodayOpen,
        StatsDay,
        StatsWeek,
    }

    private static readonly Dictionary<Slot, string> SlotNames = new()
    {
        [Slot.List] = "list",
        [Slot.More] = "more",
        [Slot.Search] = "search",
        [Slot.Open] = "open",
        [Slot.TodayMeetings] = "todayMeetings",
        [Slot.TodayOpen] = "todayOpen",
        [Slot.StatsDay] = "statsDay",
        [Slot.StatsWeek] = "statsWeek",
    };

    private static readonly Dictionary<string, Slot> SlotsByName =
        SlotNames.ToDictionary(p => p.Value, p => p.Key, StringComparer.Ordinal);

    private readonly Action<CoreCommand> _send;
    private readonly Func<RecordDocument, RecordPlayer?> _makePlayer;
    private readonly ISearchScheduler? _searchScheduler;
    private readonly Func<DateTimeOffset> _now;
    private readonly LibraryCalendar? _calendar;
    private int _sequence;
    private readonly Dictionary<Slot, string> _latest = [];
    private IDisposable? _pendingSearch;
    /// <summary>Where to put the playhead once the record being opened arrives.</summary>
    private long? _pendingSeek;
    private bool _pendingPlay;
    private RecordKind? _filter = RecordKind.Meeting;
    private string _query = "";

    /// <param name="makePlayer">Makes the player for a record with audio (the app's builds it over its audio output; null: no player).</param>
    /// <param name="searchScheduler">Waits for typing to pause before a search; null asks at once (tests).</param>
    /// <param name="now">The clock "today" and "this week" are counted from.</param>
    /// <param name="calendar">The zone and culture days are read in; null follows this PC's.</param>
    public LibraryModel(
        Action<CoreCommand> send,
        Func<RecordDocument, RecordPlayer?>? makePlayer = null,
        ISearchScheduler? searchScheduler = null,
        Func<DateTimeOffset>? now = null,
        LibraryCalendar? calendar = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        _send = send;
        _makePlayer = makePlayer ?? (_ => null);
        _searchScheduler = searchScheduler;
        _now = now ?? (() => DateTimeOffset.Now);
        _calendar = calendar;
    }

    public LibraryCalendar Calendar => _calendar ?? LibraryCalendar.Local;

    public DateTimeOffset Now() => _now();

    /// <summary>The list's filter: one kind (the chips), or null for every kind.</summary>
    public RecordKind? Filter
    {
        get => _filter;
        set
        {
            if (_filter == value)
            {
                return;
            }
            _filter = value;
            RefreshList();
        }
    }

    /// <summary>A chip was pressed: pressing the one shown shows every kind.</summary>
    public void ToggleFilter(RecordKind kind) => Filter = Filter == kind ? null : kind;

    /// <summary>What the search field holds. A non-empty query shows matches instead of the list.</summary>
    public string Query
    {
        get => _query;
        set
        {
            value ??= "";
            if (_query == value)
            {
                return;
            }
            _query = value;
            Search();
            Changed();
        }
    }

    /// <summary>The listed records, newest first by start (RecordOrder).</summary>
    public IReadOnlyList<RecordRow> Records { get; private set; } = [];

    /// <summary>Whether another page follows.</summary>
    public bool HasMore { get; private set; }

    /// <summary>The list's first page. Failed: the list could not be read (not an empty library).</summary>
    public LibraryLoad ListLoad { get; private set; }

    /// <summary>The next page: Failed shows a retry under the list.</summary>
    public LibraryLoad MoreLoad { get; private set; }

    /// <summary>Matches for the query, best first.</summary>
    public IReadOnlyList<SearchHit> Hits { get; private set; } = [];

    /// <summary>The query the hits answer.</summary>
    public string HitsQuery { get; private set; } = "";

    /// <summary>The search: Failed says it could not search, never "nothing matches".</summary>
    public LibraryLoad SearchLoad { get; private set; }

    /// <summary>The record the Library shows.</summary>
    public string? Selected { get; private set; }

    /// <summary>It, once the core has answered.</summary>
    public RecordDocument? Document { get; private set; }

    /// <summary>Why it could not be opened, when it could not.</summary>
    public string? OpenFailure { get; private set; }

    /// <summary>Plays the open record's audio.</summary>
    public RecordPlayer? Player { get; private set; }

    /// <summary>Today: the latest finished meeting, whole.</summary>
    public RecordDocument? LastMeeting { get; private set; }

    /// <summary>Today: the last meeting's question. Loaded with no meeting: there is none yet.</summary>
    public LibraryLoad LastMeetingLoad { get; private set; }

    /// <summary>Since the start of today.</summary>
    public LibraryStats? Today { get; private set; }

    /// <summary>Since the start of the week.</summary>
    public LibraryStats? Week { get; private set; }

    /// <summary>The two counts' questions. Failed: that count could not be read, which is not zero.</summary>
    public LibraryLoad TodayLoad { get; private set; }

    public LibraryLoad WeekLoad { get; private set; }

    /// <summary>What the needs-you banner reads about the far end.</summary>
    public FarEndCheck FarEnd
    {
        get
        {
            if ((Week ?? Today) is { } stats)
            {
                return new FarEndCheck.Checked(
                    stats.FarSilentMeetings, stats.FarSilentSinceUnixMs is long since ? LibraryFormat.Date(since) : null);
            }
            return TodayLoad == LibraryLoad.Failed && WeekLoad == LibraryLoad.Failed ? new FarEndCheck.Failed() : new FarEndCheck.Unknown();
        }
    }

    // Questions

    /// <summary>A new id for <paramref name="slot"/>; any earlier question of the slot is now stale.</summary>
    private string RefFor(Slot slot)
    {
        _sequence++;
        var id = string.Create(CultureInfo.InvariantCulture, $"{SlotNames[slot]}-{_sequence}");
        _latest[slot] = id;
        return id;
    }

    /// <summary>The slot of an id, whether or not it is the newest.</summary>
    private static Slot? SlotOf(string? id)
    {
        var dash = id?.LastIndexOf('-') ?? -1;
        return dash > 0 && SlotsByName.TryGetValue(id![..dash], out var slot) ? slot : null;
    }

    /// <summary>The slot an answer (or failure) with id <paramref name="reference"/> belongs to, if it is that slot's newest question.</summary>
    private Slot? Current(string? reference) =>
        SlotOf(reference) is Slot slot && _latest.TryGetValue(slot, out var latest) && latest == reference ? slot : null;

    /// <summary>
    /// Whether this model shows <paramref name="failed"/> itself (the aggregator logs the rest by
    /// name). Every library command it sent is its own, stale ones included: a stale failure
    /// changes nothing on screen and needs no log line of its own.
    /// </summary>
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1822", Justification = "The aggregator asks each model instance, as the Mac's does.")]
    public bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return SlotOf(failed.Id) is not null;
    }

    /// <summary>Reloads the list's first page.</summary>
    public void RefreshList()
    {
        if (ListLoad != LibraryLoad.Loaded)
        {
            ListLoad = LibraryLoad.Loading;
        }
        MoreLoad = LibraryLoad.Idle;
        // A page asked for under the old list is stale now: its answer must not append.
        _latest.Remove(Slot.More);
        _send(new CoreCommand.RecordsList(Filter, null, PageSize, RefFor(Slot.List)));
        Changed();
    }

    /// <summary>Loads the page after the last listed record.</summary>
    public void LoadMore()
    {
        if (!HasMore || Records.Count == 0)
        {
            return;
        }
        var last = Records[^1];
        MoreLoad = LibraryLoad.Loading;
        _send(new CoreCommand.RecordsList(Filter, new RecordCursor(last.StartedAtUnixMs, last.Record), PageSize, RefFor(Slot.More)));
        Changed();
    }

    private void Search()
    {
        _pendingSearch?.Dispose();
        _pendingSearch = null;
        var words = Query.Trim();
        if (words.Length > MaxQueryLength)
        {
            words = words[..MaxQueryLength];
        }
        if (words.Length == 0)
        {
            _latest.Remove(Slot.Search);
            Hits = [];
            HitsQuery = "";
            SearchLoad = LibraryLoad.Idle;
            return;
        }
        SearchLoad = LibraryLoad.Loading;
        if (_searchScheduler is null)
        {
            _send(new CoreCommand.RecordsSearch(words, 100, RefFor(Slot.Search)));
            return;
        }
        // A one-shot wait, cancelled by the next key, never a timer that polls.
        _pendingSearch = _searchScheduler.After(SearchDelay, () =>
        {
            _pendingSearch = null;
            _send(new CoreCommand.RecordsSearch(words, 100, RefFor(Slot.Search)));
        });
    }

    /// <summary>Everything Today shows that the library holds (what is owed is the Owed model's).</summary>
    public void RefreshToday()
    {
        if (LastMeetingLoad != LibraryLoad.Loaded)
        {
            LastMeetingLoad = LibraryLoad.Loading;
        }
        _send(new CoreCommand.RecordsList(RecordKind.Meeting, null, 10, RefFor(Slot.TodayMeetings)));
        var now = _now();
        var calendar = Calendar;
        if (TodayLoad != LibraryLoad.Loaded)
        {
            TodayLoad = LibraryLoad.Loading;
        }
        if (WeekLoad != LibraryLoad.Loaded)
        {
            WeekLoad = LibraryLoad.Loading;
        }
        _send(new CoreCommand.LibraryStats(calendar.StartOfDay(now).ToUnixTimeMilliseconds(), RefFor(Slot.StatsDay)));
        _send(new CoreCommand.LibraryStats(calendar.StartOfWeek(now).ToUnixTimeMilliseconds(), RefFor(Slot.StatsWeek)));
        Changed();
    }

    /// <summary>Shows <paramref name="record"/>; with <paramref name="seekMs"/>, puts the playhead there once it is open, and plays when <paramref name="play"/> is set.</summary>
    public void Open(string record, long? seekMs = null, bool play = false)
    {
        ArgumentNullException.ThrowIfNull(record);
        _pendingSeek = seekMs;
        _pendingPlay = play;
        if (Selected == record && Document is not null && Document.Record.Record == record)
        {
            // Already open: only move the playhead.
            if (seekMs is long ms)
            {
                PlayFrom(ms, play);
            }
            _pendingSeek = null;
            return;
        }
        Selected = record;
        Document = null;
        OpenFailure = null;
        ReplacePlayer(null);
        _send(new CoreCommand.RecordOpen(record, RefFor(Slot.Open)));
        Changed();
    }

    /// <summary>Opens the selected record again (after a failure, or when it changed).</summary>
    public void Reopen()
    {
        if (Selected is not string selected)
        {
            return;
        }
        OpenFailure = null;
        _send(new CoreCommand.RecordOpen(selected, RefFor(Slot.Open)));
        Changed();
    }

    /// <summary>
    /// Marks one of the open record's commitments done or open again. The Owed screen's model
    /// shows a failure (it lists again); the record is read again when the core says it changed.
    /// </summary>
    public void SetDone(string commitment, bool done) => _send(new CoreCommand.CommitmentSetDone(commitment, done));

    /// <summary>Puts the playhead at <paramref name="ms"/> (a chip, a line, a search hit) and plays from there.</summary>
    public void PlayFrom(long ms, bool start = true)
    {
        if (Player is not { } player)
        {
            return;
        }
        player.Seek(ms);
        if (start)
        {
            player.Play();
        }
    }

    /// <summary>The ledger line under the playhead, once the playhead has been put somewhere (read while the player bar redraws).</summary>
    public int? PlayheadLine() =>
        Player is { IsPlaced: true } player && Document is { } document ? document.LineAtPlayhead(player.PositionMs())?.Id : null;

    private void ReplacePlayer(RecordPlayer? next)
    {
        Player?.Stop();
        Player = next;
    }

    // Answers

    /// <summary>Folds in what one event says about the library.</summary>
    public void Apply(InkEvent e) => Apply([e]);

    /// <summary>Folds in what a batch of events says about the library: one refresh however many events asked for it.</summary>
    public void Apply(IEnumerable<InkEvent> batch)
    {
        ArgumentNullException.ThrowIfNull(batch);
        var libraryChanged = false;
        var recordChanged = false;
        var changed = false;
        foreach (var e in batch)
        {
            switch (e)
            {
                case LibraryRecords answer:
                    changed |= Receive(answer);
                    break;
                case LibrarySearch answer when Current(answer.Ref) == Slot.Search:
                    Hits = answer.Hits;
                    HitsQuery = answer.Query;
                    SearchLoad = LibraryLoad.Loaded;
                    changed = true;
                    break;
                case LibraryRecord answer:
                    changed |= Receive(answer);
                    break;
                case LibraryStats answer:
                    switch (Current(answer.Ref))
                    {
                        case Slot.StatsDay:
                            Today = answer;
                            TodayLoad = LibraryLoad.Loaded;
                            changed = true;
                            break;
                        case Slot.StatsWeek:
                            Week = answer;
                            WeekLoad = LibraryLoad.Loaded;
                            changed = true;
                            break;
                    }
                    break;
                case CommandFailed failed:
                    changed |= Fail(failed);
                    break;
                case CommitmentUpdated or NoteAdded or NoteUpdated or NoteDeleted:
                    // The open record may hold it: read it again.
                    recordChanged = true;
                    break;
                case MeetingFinished or DictationInserted or CoreReady or ImportFinished:
                    // A record was written or finished, or 0.2's came over: what the screens list
                    // has changed.
                    libraryChanged = true;
                    break;
            }
        }
        if (libraryChanged)
        {
            RefreshList();
            RefreshToday();
        }
        if ((libraryChanged || recordChanged) && Document is not null)
        {
            Reopen();
        }
        if (changed)
        {
            Changed();
        }
    }

    private bool Fail(CommandFailed failed)
    {
        switch (Current(failed.Id))
        {
            case Slot.List:
                ListLoad = LibraryLoad.Failed;
                return true;
            case Slot.More:
                MoreLoad = LibraryLoad.Failed;
                return true;
            case Slot.Search:
                Hits = [];
                SearchLoad = LibraryLoad.Failed;
                return true;
            case Slot.Open:
                // A record already shown stays shown: only a record never read says it failed.
                if (Document is null)
                {
                    OpenFailure = failed.Message;
                }
                return true;
            case Slot.TodayMeetings or Slot.TodayOpen:
                LastMeetingLoad = LibraryLoad.Failed;
                return true;
            case Slot.StatsDay:
                // An old count is not shown as today's: it is gone, and the failure is said.
                Today = null;
                TodayLoad = LibraryLoad.Failed;
                return true;
            case Slot.StatsWeek:
                Week = null;
                WeekLoad = LibraryLoad.Failed;
                return true;
            default:
                return false;
        }
    }

    private bool Receive(LibraryRecords answer)
    {
        switch (Current(answer.Ref))
        {
            case Slot.List:
                Records = RecordOrder.NewestFirst(answer.Records);
                HasMore = answer.More;
                ListLoad = LibraryLoad.Loaded;
                return true;
            case Slot.More:
                Records = RecordOrder.NewestFirst(Records.Concat(answer.Records));
                HasMore = answer.More;
                MoreLoad = LibraryLoad.Loaded;
                return true;
            case Slot.TodayMeetings:
                if (RecordOrder.LatestFinishedMeeting(answer.Records) is not { } latest)
                {
                    LastMeeting = null;
                    LastMeetingLoad = LibraryLoad.Loaded;
                    return true;
                }
                _send(new CoreCommand.RecordOpen(latest.Record, RefFor(Slot.TodayOpen)));
                return false;
            default:
                return false;
        }
    }

    private bool Receive(LibraryRecord answer)
    {
        switch (Current(answer.Ref))
        {
            case Slot.Open:
                var document = new RecordDocument(answer);
                var same = Document?.Record.Record == document.Record.Record;
                Document = document;
                OpenFailure = null;
                if (!same || Player is null)
                {
                    ReplacePlayer(document.Chunks.Count == 0 ? null : _makePlayer(document));
                }
                if (_pendingSeek is long seek)
                {
                    PlayFrom(seek, _pendingPlay);
                    _pendingSeek = null;
                }
                return true;
            case Slot.TodayOpen:
                LastMeeting = new RecordDocument(answer);
                LastMeetingLoad = LibraryLoad.Loaded;
                return true;
            default:
                return false;
        }
    }

    // What the Library screen says (the Mac's LibraryScreen).

    public const string SearchPrompt = "Search everything said";
    public const string NothingSelectedTitle = "Choose a record";
    public const string NothingSelectedDetail = "Its notes, transcript and summary open here, with its audio.";
    public const string ListFailedText = "Couldn't load the library";
    public const string OpenFailedTitle = "This record can't be opened";
    public const string TryAgainText = "Try again";

    /// <summary>Whether the column shows matches instead of the list.</summary>
    public bool IsSearching => Query.Trim().Length > 0;

    /// <summary>The column's heading.</summary>
    public string ColumnTitle => IsSearching ? "Search" : "Library";

    /// <summary>The count beside the heading: <c>100+</c> while more records follow.</summary>
    public string CountText => IsSearching
        ? Hits.Count.ToString(CultureInfo.InvariantCulture)
        : string.Create(CultureInfo.InvariantCulture, $"{Records.Count}{(HasMore ? "+" : "")}");

    /// <summary>The count as a screen reader reads it.</summary>
    public string CountLabel => IsSearching
        ? string.Create(CultureInfo.InvariantCulture, $"{Hits.Count} matches")
        : string.Create(CultureInfo.InvariantCulture, $"{Records.Count} records");

    /// <summary>What an empty list says, for the filter shown.</summary>
    public (string Title, string Detail) EmptyText => Filter switch
    {
        RecordKind.Meeting => ("No meetings yet", "When you join a call, Inkwell records both sides and the record lands here."),
        RecordKind.Dictation => ("No dictations yet", "Each dictation lands here, with the words it typed."),
        RecordKind.FileImport => ("No imported files yet", "Audio and video files you import land here, transcribed."),
        _ => ("Nothing here yet", "Meetings, dictations and imported files land here."),
    };

    /// <summary>Whether the list shows its empty text (answered, and empty; never while failed).</summary>
    public bool ShowsEmpty => ListLoad == LibraryLoad.Loaded && Records.Count == 0;

    /// <summary>The button under the list while more records follow.</summary>
    public string MoreText => MoreLoad == LibraryLoad.Failed ? "Couldn't load older records. Try again" : "Show older";

    /// <summary>What the search says while it shows no matches.</summary>
    public string SearchStatus => SearchLoad switch
    {
        LibraryLoad.Failed => "Couldn't search the library.",
        LibraryLoad.Loaded => $"Nothing said matches “{HitsQuery}”.",
        _ => "Searching…",
    };

    /// <summary>A chip's hint for a screen reader.</summary>
    public string ChipHint(KindChip chip)
    {
        ArgumentNullException.ThrowIfNull(chip);
        return Filter == chip.Kind ? "Shows every kind" : $"Shows only {chip.InSentence}";
    }

    /// <summary>A match's title.</summary>
    public static string HitTitle(SearchHit hit)
    {
        ArgumentNullException.ThrowIfNull(hit);
        return hit.Title ?? "Untitled record";
    }
}
