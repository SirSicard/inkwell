// What the Today, Library and Record screens show of the library, built from the core's answers
// to the library's commands (CoreCommand.recordsList and the rest). The screens ask by calling the
// refresh methods; answers arrive as events in the same batches the CoreStore folds, on the main
// actor. What is owed and the permissions are the Owed and Settings models' (ScreenModels): Today
// reads them there.
//
// Each question goes out with an id naming its slot (the list, a search, the open record, ...)
// and a number, and the answer echoes it as `ref`. Only the answer to a slot's newest question is
// kept: typing a search fast, or clicking through records, never shows an older answer over a
// newer one. A question that fails is a `command.failed` with that id, and the slot then reads as
// "could not load", never as an empty library.
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
        case statsDay
        case statsWeek
        /// A name given to one of the open record's speakers.
        case speaker
        /// A record the user deleted.
        case delete
    }

    /// Where an answer is: asked and not answered, answered, or failed.
    enum Load: Equatable, Sendable {
        case idle
        case loading
        case loaded
        case failed
    }

    /// The list's filter: one kind (the canvas's chips), or nil for every kind (All, where the
    /// Library opens).
    var filter: RecordKind? = nil {
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
    /// The list's first page. `.failed`: the list could not be read (not an empty library).
    private(set) var listLoad: Load = .idle
    /// The next page: `.failed` shows a retry under the list.
    private(set) var moreLoad: Load = .idle
    /// Matches for `query`, best first.
    private(set) var hits: [SearchHit] = []
    /// The query `hits` answer.
    private(set) var hitsQuery = ""
    /// The search: `.failed` says it could not search, never "nothing matches".
    private(set) var searchLoad: Load = .idle

    /// The record the Library shows.
    private(set) var selected: String?
    /// It, once the core has answered.
    private(set) var document: RecordDocument?
    /// Why it could not be opened, when it could not.
    private(set) var openFailure: String?
    /// Plays the open record's audio.
    private(set) var player: RecordPlayer?
    /// Why the last name given to a speaker was not saved (the core's words), until the next try.
    private(set) var namingFailure: String?
    /// Why the last record the user deleted was not deleted (the core's words), until the next try.
    private(set) var deleteFailure: String?
    /// What the last deletion left on this Mac, when it left anything: its words not yet cleared
    /// from the library's files, or its recording.
    private(set) var deletionNote: String?

    /// Today: the latest finished meeting, whole.
    private(set) var lastMeeting: RecordDocument?
    /// Today: the last meeting's question. `.loaded` with no meeting: there is none yet.
    private(set) var lastMeetingLoad: Load = .idle
    /// Since the start of today.
    private(set) var today: LibraryStats?
    /// Since the start of the week.
    private(set) var week: LibraryStats?
    /// The two counts' questions. `.failed`: that count could not be read, which is not zero.
    private(set) var todayLoad: Load = .idle
    private(set) var weekLoad: Load = .idle

    /// Whether the far end recorded nothing lately, as far as the counts can tell.
    enum FarEndCheck: Equatable, Sendable {
        /// Not answered yet: no guess either way.
        case unknown
        /// The counts could not be read: said, never taken as "none".
        case failed
        /// The newest meetings in a row that kept only the user's voice, and since when.
        case checked(meetings: Int64, since: Date?)
    }

    /// What the needs-you banner reads about the far end.
    var farEnd: FarEndCheck {
        if let stats = week ?? today {
            return .checked(
                meetings: stats.farSilentMeetings,
                since: stats.farSilentSinceUnixMs.map(LibraryFormat.date(unixMs:)))
        }
        return todayLoad == .failed && weekLoad == .failed ? .failed : .unknown
    }

    /// Records per page.
    static let pageSize = 100
    /// The most characters a search asks with.
    static let maxQueryLength = 200

    /// Makes the player for a record with audio (tests pass one that renders offline).
    @ObservationIgnored var makePlayer: (RecordDocument) -> RecordPlayer? = { RecordPlayer(document: $0) }
    /// The calendar and clock the stats' "today" and "this week" are counted in.
    @ObservationIgnored var calendar = Calendar.autoupdatingCurrent
    @ObservationIgnored var now: () -> Date = Date.init
    /// How long typing must pause before a search is asked: one question per pause, not one per
    /// key. A one-shot wait, cancelled by the next key, never a timer that polls.
    @ObservationIgnored var searchDelay: Duration = .milliseconds(250)
    @ObservationIgnored private var pendingSearch: Task<Void, Never>?

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var sequence = 0
    @ObservationIgnored private var latest: [Slot: String] = [:]
    /// The record the last `record.delete` asked about: its failure is shown only on it.
    @ObservationIgnored private var deleting: String?
    /// The names sent for the open record's speakers, by label: what the store holds once each is
    /// saved, before the record is read again. Forgotten when one fails, and with the record.
    @ObservationIgnored private var sentNames: [String: String] = [:]
    /// Where to put the playhead once the record being opened arrives.
    @ObservationIgnored private var pendingSeek: Int64?
    @ObservationIgnored private var pendingPlay = false

    init(send: @escaping SendCommand) {
        self.send = send
    }

    // MARK: Questions

    /// A new id for `slot`; any earlier question of the slot is now stale.
    private func ref(for slot: Slot) -> String {
        sequence += 1
        let id = "\(slot.rawValue)-\(sequence)"
        latest[slot] = id
        return id
    }

    /// The slot an answer (or failure) with id `ref` belongs to, if it is that slot's newest
    /// question.
    private func current(_ ref: String?) -> Slot? {
        guard let ref, let dash = ref.lastIndex(of: "-"),
            let slot = Slot(rawValue: String(ref[..<dash])),
            latest[slot] == ref
        else { return nil }
        return slot
    }

    /// Whether this model shows `failed` itself (the controller logs the rest by name). Every
    /// library command it sent is its own, stale ones included: a stale failure changes nothing on
    /// screen and needs no log line of its own.
    func handles(_ failed: CommandFailed) -> Bool {
        guard let id = failed.id, let dash = id.lastIndex(of: "-"),
            Slot(rawValue: String(id[..<dash])) != nil
        else { return false }
        return true
    }

    /// Reloads the list's first page.
    func refreshList() {
        if listLoad != .loaded { listLoad = .loading }
        moreLoad = .idle
        // A page asked for under the old list is stale now: its answer must not append.
        latest[.more] = nil
        send(.recordsList(kind: filter, before: nil, limit: Self.pageSize, ref: ref(for: .list)))
    }

    /// Loads the page after the last listed record.
    func loadMore() {
        guard hasMore, let last = records.last else { return }
        moreLoad = .loading
        send(.recordsList(
            kind: filter, before: .init(startedAtUnixMs: last.startedAtUnixMs, record: last.record),
            limit: Self.pageSize, ref: ref(for: .more)))
    }

    private func search() {
        pendingSearch?.cancel()
        pendingSearch = nil
        let words = String(query.trimmingCharacters(in: .whitespacesAndNewlines).prefix(Self.maxQueryLength))
        if words.isEmpty {
            latest[.search] = nil
            hits = []
            hitsQuery = ""
            searchLoad = .idle
            return
        }
        searchLoad = .loading
        if searchDelay == .zero {
            send(.recordsSearch(query: words, limit: 100, ref: ref(for: .search)))
            return
        }
        let delay = searchDelay
        pendingSearch = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled, let self else { return }
            self.send(.recordsSearch(query: words, limit: 100, ref: self.ref(for: .search)))
        }
    }

    /// Everything Today shows that the library holds (what is owed is OwedModel's).
    func refreshToday() {
        if lastMeetingLoad != .loaded { lastMeetingLoad = .loading }
        send(.recordsList(kind: .meeting, before: nil, limit: 10, ref: ref(for: .todayMeetings)))
        let now = now()
        let dayStart = calendar.startOfDay(for: now)
        let weekStart = calendar.dateInterval(of: .weekOfYear, for: now)?.start ?? dayStart
        if todayLoad != .loaded { todayLoad = .loading }
        if weekLoad != .loaded { weekLoad = .loading }
        send(.libraryStats(sinceUnixMs: Int64(dayStart.timeIntervalSince1970 * 1000), ref: ref(for: .statsDay)))
        send(.libraryStats(sinceUnixMs: Int64(weekStart.timeIntervalSince1970 * 1000), ref: ref(for: .statsWeek)))
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
        namingFailure = nil
        deleteFailure = nil
        deletionNote = nil
        sentNames = [:]
        replacePlayer(nil)
        send(.recordOpen(record: record, ref: ref(for: .open)))
    }

    /// Opens the selected record again (after a failure, or when it changed).
    func reopen() {
        guard let selected else { return }
        openFailure = nil
        send(.recordOpen(record: selected, ref: ref(for: .open)))
    }

    /// Marks one of the open record's commitments done or open again. The Owed screen's model
    /// shows a failure (it lists again); the record is read again when the core says it changed.
    func setDone(_ commitment: String, _ done: Bool) {
        send(.commitmentSetDone(id: commitment, done: done))
    }

    /// The longest name a speaker takes, in Unicode scalars, as the core counts it
    /// (MAX_SPEAKER_NAME_CHARS): an emoji or an accent written as two scalars counts two.
    static let maxSpeakerName = 80

    /// A name's length as the core counts it, once on one line.
    static func nameLength(_ name: String) -> Int {
        oneLine(name).unicodeScalars.count
    }

    /// Names one of the open record's far-end speakers, by the diarizer's label, as the user typed
    /// it: on one line (a pasted line break is a space), trimmed, and empty clears the name (the
    /// speaker reads as "Speaker N" again). Nothing is sent when nothing changed, or for a label
    /// the record's far end does not have (the mic is the user, never renamed). The record is read
    /// again when the core says it is saved; a name the core refuses (too long) is said, as any
    /// failure (`namingFailure`).
    func nameSpeaker(_ label: String, _ name: String) {
        guard let document, let speaker = document.speaker(labelled: label) else { return }
        let name = Self.oneLine(name)
        // Unchanged from what was last sent, or else from what the record says: a rename sent
        // since the record was read makes the record's name stale.
        guard name != (sentNames[label] ?? speaker.name ?? "") else { return }
        namingFailure = nil
        sentNames[label] = name
        send(.speakerName(record: document.record.record, speaker: label, name: name, ref: ref(for: .speaker)))
    }

    /// Deletes `record` whole: asked only once the user confirmed (`deletionWarning`). Nothing
    /// moves until the core says it is gone (`record.deleted`); a refusal (a record still being
    /// recorded or finished) is said (`deleteFailure`).
    func deleteRecord(_ record: String) {
        deleteFailure = nil
        deleting = record
        send(.recordDelete(record: record, ref: ref(for: .delete)))
    }

    /// What the confirmation says goes with `record`, and that it can't be undone.
    static func deletionWarning(for record: RecordRow) -> String {
        let what = switch record.kind {
        case .meeting:
            "Its audio, transcript, notes and summary, and what's owed from it, are deleted from this Mac."
        case .dictation:
            "Its words are deleted from this Mac."
        case .fileImport:
            "Its transcript, notes and summary, and what's owed from it, are deleted from this Mac. The file you imported stays where it is."
        }
        return what + " This can't be undone."
    }

    /// `text` on one line: every run of spaces, line breaks and control characters (the core
    /// refuses those) is one space, and none at either end.
    static func oneLine(_ text: String) -> String {
        text.split(whereSeparator: { character in
            character.isWhitespace
                || character.unicodeScalars.contains { $0.properties.generalCategory == .control }
        })
        .joined(separator: " ")
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
        var libraryChanged = false
        var recordChanged = false
        for event in batch {
            switch event {
            case .libraryRecords(let answer):
                receive(answer)
            case .librarySearch(let answer):
                if current(answer.ref) == .search {
                    hits = answer.hits
                    hitsQuery = answer.query
                    searchLoad = .loaded
                }
            case .libraryRecord(let answer):
                receive(answer)
            case .libraryStats(let answer):
                switch current(answer.ref) {
                case .statsDay:
                    today = answer
                    todayLoad = .loaded
                case .statsWeek:
                    week = answer
                    weekLoad = .loaded
                default: break
                }
            case .commandFailed(let failed):
                fail(failed)
            case .commitmentUpdated, .noteAdded, .noteUpdated, .noteDeleted:
                // The open record may hold it: read it again.
                recordChanged = true
            case .recordDeleted(let deleted):
                removed(deleted)
                // Today's counts and its last meeting may have changed.
                libraryChanged = true
            case .speakerNamed(let named):
                // The name shows wherever that record does: the open record, and Today's last
                // meeting.
                if document?.record.record == named.record {
                    recordChanged = true
                    if current(named.ref) == .speaker { namingFailure = nil }
                }
                if lastMeeting?.record.record == named.record {
                    send(.recordOpen(record: named.record, ref: ref(for: .todayOpen)))
                }
            case .meetingFinished, .dictationInserted, .coreReady, .importFinished:
                // A record was written or finished, or 0.2's came over: what the screens list has
                // changed.
                libraryChanged = true
            default:
                break
            }
        }
        if libraryChanged {
            refreshList()
            refreshToday()
        }
        if (libraryChanged || recordChanged), document != nil {
            reopen()
        }
    }

    private func fail(_ failed: CommandFailed) {
        switch current(failed.id) {
        case .list:
            listLoad = .failed
        case .more:
            moreLoad = .failed
        case .search:
            hits = []
            searchLoad = .failed
        case .open:
            // A record already shown stays shown: only a record never read says it failed.
            if document == nil {
                openFailure = failed.message
            }
        case .todayMeetings, .todayOpen:
            lastMeetingLoad = .failed
        case .statsDay:
            // An old count is not shown as today's: it is gone, and the failure is said.
            today = nil
            todayLoad = .failed
        case .statsWeek:
            week = nil
            weekLoad = .failed
        case .delete:
            // Only on the record it was about: another may be open by now.
            if selected == deleting { deleteFailure = failed.message }
        case .speaker:
            namingFailure = failed.message
            // What was sent did not stick: the record's names are what stands.
            sentNames = [:]
        default:
            break
        }
    }

    /// A record is gone: out of the list and the matches, out of Today, and the selection moves to
    /// the record below it in what the column shows (the matches while searching, else the list;
    /// above when it was the last), or to none. What it left behind is said until the next record
    /// is opened.
    private func removed(_ deleted: RecordDeleted) {
        let gone = deleted.record
        let searching = !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        func shown() -> [String] {
            var seen = Set<String>()
            return (searching ? hits.map(\.record) : records.map(\.record)).filter { seen.insert($0).inserted }
        }
        let index = shown().firstIndex(of: gone)
        records.removeAll { $0.record == gone }
        hits.removeAll { $0.record == gone }
        if lastMeeting?.record.record == gone {
            lastMeeting = nil
            // Asked for again (libraryChanged): not "no meetings yet" meanwhile.
            lastMeetingLoad = .loading
        }
        if selected == gone {
            let rest = shown()
            if let index, !rest.isEmpty {
                open(rest[min(index, rest.count - 1)])
            } else {
                selected = nil
                document = nil
                openFailure = nil
                pendingSeek = nil
                replacePlayer(nil)
            }
        }
        var left: [String] = []
        if !deleted.scrubbed {
            left.append("its words are still in the library's files while another app reads them, and Inkwell clears them as soon as it can")
        }
        if deleted.audioLeft {
            left.append("its recording couldn't be removed from the library's folder")
        }
        deletionNote = left.isEmpty ? nil : "Deleted, but " + left.joined(separator: ", and ") + "."
    }

    private func receive(_ answer: LibraryRecords) {
        switch current(answer.ref) {
        case .list:
            records = RecordOrder.newestFirst(answer.records)
            hasMore = answer.more
            listLoad = .loaded
        case .more:
            records = RecordOrder.newestFirst(records + answer.records)
            hasMore = answer.more
            moreLoad = .loaded
        case .todayMeetings:
            guard let latest = RecordOrder.latestFinishedMeeting(answer.records) else {
                lastMeeting = nil
                lastMeetingLoad = .loaded
                return
            }
            send(.recordOpen(record: latest.record, ref: ref(for: .todayOpen)))
        default:
            break
        }
    }

    private func receive(_ answer: LibraryRecord) {
        switch current(answer.ref) {
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
            lastMeetingLoad = .loaded
        default:
            break
        }
    }
}
