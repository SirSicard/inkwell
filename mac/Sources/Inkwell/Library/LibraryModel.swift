// What the Today, Library and Record screens show of the library, built from the core's answers
// to library queries (LibraryCommand). The screens ask by calling the refresh methods; answers
// arrive as events in the same batches the CoreStore folds, on the main actor.
//
// Each question goes out with an id naming its slot (the list, a search, the open record, ...)
// and a number. Only the answer to a slot's newest question is kept: typing a search fast, or
// clicking through records, never shows an older answer over a newer one.
import Foundation
import InkBridge
import Observation

@MainActor
@Observable
final class LibraryModel {
    /// What a question is for.
    enum Slot: String, CaseIterable, Sendable {
        case list
        case more
        case search
        case open
        case todayMeetings
        case todayOpen
        case owed
        case statsDay
        case statsWeek
        case permissions
        case done
    }

    /// The list's filter: one kind (the canvas's chips), or nil for every kind.
    var filter: RecordKind? = .meeting {
        didSet {
            if filter != oldValue { refreshList() }
        }
    }

    /// What the search field holds. A non-empty query shows matches instead of the list.
    var query = "" {
        didSet {
            if query != oldValue { search() }
        }
    }

    /// The listed records, newest first by start (RecordOrder).
    private(set) var records: [RecordRow] = []
    /// Whether another page follows.
    private(set) var hasMore = false
    /// Whether the list has been answered at least once (an empty list is then truly empty).
    private(set) var listLoaded = false
    /// Matches for `query`, best first.
    private(set) var hits: [SearchHit] = []
    /// The query `hits` answer.
    private(set) var hitsQuery = ""

    /// The record the Library shows.
    private(set) var selected: String?
    /// It, once the core has answered.
    private(set) var document: RecordDocument?
    /// Why it could not be opened, when it could not.
    private(set) var openFailure: String?
    /// Plays the open record's audio.
    private(set) var player: RecordPlayer?

    /// Today: the latest finished meeting, whole.
    private(set) var lastMeeting: RecordDocument?
    /// Today: whether its question has been answered (there may be none).
    private(set) var lastMeetingLoaded = false
    /// Open commitments, soonest due first, as many as Today shows.
    private(set) var owed: [OwedItem] = []
    /// How many are open in all.
    private(set) var owedTotal: Int64 = 0
    /// Since the start of today.
    private(set) var today: LibraryStats?
    /// Since the start of the week.
    private(set) var week: LibraryStats?
    /// The permissions, as last checked.
    private(set) var permissions: PermissionsChecked?

    /// How many open commitments Today lists.
    static let owedOnToday = 3
    /// Records per page.
    static let pageSize = 100

    /// Sends a command's JSON to the core (CoreController.send(json:)).
    @ObservationIgnored var send: (String) -> Void = { _ in }
    /// Makes the player for a record with audio (tests pass one that renders offline).
    @ObservationIgnored var makePlayer: (RecordDocument) -> RecordPlayer? = { RecordPlayer(document: $0) }
    /// The calendar and clock the stats' "today" and "this week" are counted in.
    @ObservationIgnored var calendar = Calendar.autoupdatingCurrent
    @ObservationIgnored var now: () -> Date = Date.init

    @ObservationIgnored private var sequence = 0
    @ObservationIgnored private var latest: [Slot: String] = [:]
    /// Where to put the playhead once the record being opened arrives.
    @ObservationIgnored private var pendingSeek: Int64?
    @ObservationIgnored private var pendingPlay = false

    init() {}

    // MARK: Questions

    /// Sends `command` for `slot`; any earlier question of the slot is now stale.
    private func ask(_ command: LibraryCommand, for slot: Slot) {
        sequence += 1
        let id = "\(slot.rawValue)-\(sequence)"
        latest[slot] = id
        send(command.json(id: id))
    }

    /// The slot an answer with `request` id belongs to, if it is that slot's newest question.
    private func current(_ request: String?) -> Slot? {
        guard let request, let dash = request.lastIndex(of: "-"),
            let slot = Slot(rawValue: String(request[..<dash])),
            latest[slot] == request
        else { return nil }
        return slot
    }

    /// Reloads the list's first page.
    func refreshList() {
        ask(.list(kind: filter, before: nil, limit: Self.pageSize), for: .list)
    }

    /// Loads the page after the last listed record.
    func loadMore() {
        guard hasMore, let last = records.last else { return }
        ask(.list(kind: filter, before: .init(startedAtUnixMs: last.startedAtUnixMs, record: last.record),
                  limit: Self.pageSize), for: .more)
    }

    private func search() {
        let words = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if words.isEmpty {
            latest[.search] = nil
            hits = []
            hitsQuery = ""
            return
        }
        ask(.search(query: words, limit: 100), for: .search)
    }

    /// Everything Today shows that the library holds.
    func refreshToday() {
        ask(.list(kind: .meeting, before: nil, limit: 10), for: .todayMeetings)
        ask(.owed(limit: Self.owedOnToday), for: .owed)
        let now = now()
        let dayStart = calendar.startOfDay(for: now)
        let weekStart = calendar.dateInterval(of: .weekOfYear, for: now)?.start ?? dayStart
        ask(.stats(sinceUnixMs: Int64(dayStart.timeIntervalSince1970 * 1000)), for: .statsDay)
        ask(.stats(sinceUnixMs: Int64(weekStart.timeIntervalSince1970 * 1000)), for: .statsWeek)
    }

    /// Checks the permissions again (Today on appear, and when the app comes back to the front).
    func refreshPermissions() {
        ask(.permissions, for: .permissions)
    }

    /// Shows `record`; with `seekMs`, puts the playhead there once it is open, and plays when
    /// `play` is set.
    func open(_ record: String, seekMs: Int64? = nil, play: Bool = false) {
        pendingSeek = seekMs
        pendingPlay = play
        if selected == record, let document, document.record.record == record {
            // Already open: only move the playhead.
            if let seekMs { playFrom(seekMs, start: play) }
            pendingSeek = nil
            return
        }
        selected = record
        document = nil
        openFailure = nil
        replacePlayer(nil)
        ask(.open(record: record), for: .open)
    }

    /// Marks a commitment done or open again.
    func setDone(_ commitment: String, _ done: Bool) {
        ask(.setDone(commitment: commitment, done: done), for: .done)
    }

    /// Puts the playhead at `ms` (a chip, a line, a search hit) and plays from there.
    func playFrom(_ ms: Int64, start: Bool = true) {
        guard let player else { return }
        player.seek(toMs: ms)
        if start { player.play() }
    }

    private func replacePlayer(_ new: RecordPlayer?) {
        player?.stop()
        player = new
    }

    // MARK: Answers

    /// Folds in what a batch of events says about the library.
    func apply(_ batch: [InkEvent]) {
        var changed = false
        for event in batch {
            switch event {
            case .libraryRecords(let answer):
                receive(answer)
            case .librarySearch(let answer):
                if current(answer.request) == .search {
                    hits = answer.hits
                    hitsQuery = answer.query
                }
            case .libraryRecord(let answer):
                receive(answer)
            case .libraryOwed(let answer):
                if current(answer.request) == .owed {
                    owed = answer.commitments
                    owedTotal = answer.total
                }
            case .libraryCommitmentDone:
                // Whatever lists commitments is asked again below.
                changed = true
            case .libraryStats(let answer):
                switch current(answer.request) {
                case .statsDay: today = answer
                case .statsWeek: week = answer
                default: break
                }
            case .permissionsChecked(let answer):
                if current(answer.request) == .permissions {
                    permissions = answer
                }
            case .commandFailed(let failed):
                if current(failed.id) == .open {
                    openFailure = failed.message
                }
            case .meetingFinished, .dictationInserted:
                // A record was written or finished: what the screens list has changed.
                changed = true
            case .coreReady:
                changed = true
            default:
                break
            }
        }
        if changed {
            refreshList()
            refreshToday()
            if let selected, document != nil {
                // The open record may be the one that changed (a commitment marked done).
                ask(.open(record: selected), for: .open)
            }
        }
    }

    private func receive(_ answer: LibraryRecords) {
        switch current(answer.request) {
        case .list:
            records = RecordOrder.newestFirst(answer.records)
            hasMore = answer.more
            listLoaded = true
        case .more:
            records = RecordOrder.newestFirst(records + answer.records)
            hasMore = answer.more
        case .todayMeetings:
            guard let latest = RecordOrder.latestFinishedMeeting(answer.records) else {
                lastMeeting = nil
                lastMeetingLoaded = true
                return
            }
            ask(.open(record: latest.record), for: .todayOpen)
        default:
            break
        }
    }

    private func receive(_ answer: LibraryRecord) {
        switch current(answer.request) {
        case .open:
            let document = RecordDocument(answer)
            let same = self.document?.record.record == document.record.record
            self.document = document
            openFailure = nil
            if !same || player == nil {
                replacePlayer(document.chunks.isEmpty ? nil : makePlayer(document))
            }
            if let seek = pendingSeek {
                playFrom(seek, start: pendingPlay)
                pendingSeek = nil
            }
        case .todayOpen:
            lastMeeting = RecordDocument(answer)
            lastMeetingLoaded = true
        default:
            break
        }
    }
}
