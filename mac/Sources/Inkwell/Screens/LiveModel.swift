// Live: a meeting as it happens. Three parts, each with its own rule:
//
// - The ledger: what is being said. Finals are dry (settled, in the record) and partials are wet
//   (the engine's current guess, grey and italic, replaced by the next one). Partials are shown
//   and never stored: they live only in the core store's live meeting, until the next partial or
//   final replaces them (architecture rule 4).
// - Notes: the user types as they listen. Each paragraph is one note, stamped with where in the
//   meeting it was started, and saved to the record (note.add) when the user moves on from it:
//   another line, the editor losing focus, or the meeting ending. Edits to a saved line update it;
//   emptying one deletes it. Nothing is saved on a timer.
// - Ask, with the question stack: the far end's questions (up to four, newest first) are stacked
//   so one can be answered with a key press, and anything else can be asked. The core answers
//   (meeting.ask) from the transcript so far, with the language model the shell registered; the
//   answer is the model's words, shown as plain text only, and a failure says so in words.
import Foundation
import InkBridge
import Observation

/// One line of the ledger.
struct LiveLine: Equatable, Identifiable {
    let id: String
    let channel: Channel
    /// Where in the meeting it started, ms; nil for a partial.
    let atMs: Int64?
    let text: String
    /// Still settling: a partial.
    let wet: Bool

    /// Everything the core has for the live meeting: the finals in the order they were said (each
    /// side settles at its own pace, so they arrive out of order), then each side's partial.
    static func ledger(_ meeting: CoreStore.LiveMeeting) -> [LiveLine] {
        var lines = meeting.finals.enumerated().map { index, final in
            LiveLine(id: "final-\(index)", channel: final.channel, atMs: final.startMs, text: final.text, wet: false)
        }
        // Stable: lines said at the same moment keep their arrival order.
        lines = lines.enumerated()
            .sorted { ($0.element.atMs ?? 0, $0.offset) < ($1.element.atMs ?? 0, $1.offset) }
            .map(\.element)
        for channel in [Channel.mic, .far] {
            if let partial = meeting.partials[channel], !partial.trimmingCharacters(in: .whitespaces).isEmpty {
                lines.append(LiveLine(id: "partial-\(channel.rawValue)", channel: channel, atMs: nil, text: partial, wet: true))
            }
        }
        return lines
    }
}

/// "12:41": minutes and seconds into the meeting (hours when past one).
func liveClock(ms: Int64) -> String {
    let seconds = max(0, ms / 1_000)
    let (h, m, s) = (seconds / 3_600, (seconds % 3_600) / 60, seconds % 60)
    return h > 0
        ? String(format: "%d:%02d:%02d", h, m, s)
        : String(format: "%d:%02d", m, s)
}

// MARK: - Notes

/// One paragraph of the notes and what the core has of it.
struct LiveNoteLine: Equatable {
    /// Local and stable while the line lives.
    let key: Int
    var text: String
    /// Where in the meeting it was started, ms: set on its first character.
    var atMs: UInt64?
    /// The core's id, once added. Kept while a delete is on its way, and after a delete failed.
    var note: String?
    /// The text the core has, or has been sent. Cleared when a save fails, so the next save tries
    /// again.
    var saved: String?
    /// note.add is on its way.
    var adding = false
    /// note.delete is on its way (the line was emptied). Nothing else is sent for the line until
    /// it is answered: a retype then updates the note (the delete failed) or adds a new one.
    var deleting = false

    var isBlank: Bool { text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
}

/// The notes of one live meeting: paragraphs in, note commands out.
///
/// Every command carries a ref naming the record, the line and the kind of command, and every
/// answer (note.added, note.updated, note.deleted, or command.failed with the ref as its id) is
/// matched back to its line. A failed save leaves the line unsaved, to be tried again when the
/// user next moves on; a failed delete keeps the note's id, so the line is never added twice.
struct LiveNotesDraft {
    private(set) var lines: [LiveNoteLine] = [LiveNoteLine(key: 0, text: "")]
    private var nextKey = 1
    /// The line with the caret; it is saved when the user moves on.
    private var editingKey: Int?
    /// Notes of lines that no longer exist, by line key: their delete is on its way.
    private var orphans: [Int: String] = [:]
    /// Notes of lines that no longer exist whose delete failed: tried again on the next flush.
    private var orphansToRetry: [Int: String] = [:]
    /// The meeting's record.
    let record: String

    init(record: String) {
        self.record = record
    }

    /// What a command for a line is.
    enum RefKind: String {
        case add = ""
        case update = ":update"
        case delete = ":delete"
    }

    /// The ref of `kind` for line `key`: the record, the line, and the kind.
    func ref(_ key: Int, _ kind: RefKind = .add) -> String { "\(record):line:\(key)\(kind.rawValue)" }

    /// The line key in `ref` if it is a ref of `kind` of this draft's record.
    private func key(_ ref: String, _ kind: RefKind) -> Int? {
        let prefix = "\(record):line:"
        guard ref.hasPrefix(prefix), ref.hasSuffix(kind.rawValue) else { return nil }
        return Int(ref.dropFirst(prefix.count).dropLast(kind.rawValue.count))
    }

    /// The editor's paragraphs after an edit, with the caret's paragraph (nil: no caret). Returns
    /// what to send.
    mutating func edited(_ paragraphs: [String], caret: Int?, nowMs: UInt64) -> [CoreCommand] {
        var commands = align(to: paragraphs.isEmpty ? [""] : paragraphs)
        for i in lines.indices where lines[i].atMs == nil && !lines[i].isBlank {
            lines[i].atMs = nowMs
        }
        editingKey = caret.flatMap { lines.indices.contains($0) ? lines[$0].key : nil }
        for i in lines.indices where lines[i].key != editingKey {
            commands += persist(i)
        }
        return commands
    }

    /// Saves every line, the one being edited included (focus left, the meeting ended, or the app
    /// quitting), and tries again the deletes that failed.
    mutating func flush() -> [CoreCommand] {
        editingKey = nil
        var commands = lines.indices.flatMap { persist($0) }
        for (key, note) in orphansToRetry.sorted(by: { $0.key < $1.key }) {
            orphans[key] = note
            commands.append(.noteDelete(note: note, ref: ref(key, .delete)))
        }
        orphansToRetry = [:]
        return commands
    }

    /// The core added the note sent for `ref`.
    mutating func added(ref added: String, note: String) -> [CoreCommand] {
        guard let key = key(added, .add) else { return [] }
        guard let i = lines.firstIndex(where: { $0.key == key }) else {
            // Its line was deleted while the add was on its way.
            orphans[key] = note
            return [.noteDelete(note: note, ref: ref(key, .delete))]
        }
        lines[i].note = note
        lines[i].adding = false
        return lines[i].key == editingKey ? [] : persist(i)
    }

    /// The core refused an add, update or delete: the line is rolled back to what the core has.
    mutating func failed(ref failed: String) -> [CoreCommand] {
        if let key = key(failed, .add), let i = lines.firstIndex(where: { $0.key == key }) {
            // No note: tried again on the next save.
            lines[i].adding = false
            lines[i].saved = nil
        } else if let key = key(failed, .update), let i = lines.firstIndex(where: { $0.key == key }) {
            // The core has the older text: tried again on the next save.
            lines[i].saved = nil
        } else if let key = key(failed, .delete) {
            if let i = lines.firstIndex(where: { $0.key == key }) {
                // The note is still there: a retyped line updates it, and a still-empty line
                // deletes it again on the next save.
                lines[i].deleting = false
                lines[i].saved = nil
                if !lines[i].isBlank && lines[i].key != editingKey {
                    return persist(i)
                }
            } else if let note = orphans.removeValue(forKey: key) {
                orphansToRetry[key] = note
            }
        }
        return []
    }

    /// The core deleted the note sent for `ref`.
    mutating func deleted(ref deleted: String) -> [CoreCommand] {
        guard let key = key(deleted, .delete) else { return [] }
        guard let i = lines.firstIndex(where: { $0.key == key }) else {
            orphans[key] = nil
            return []
        }
        lines[i].deleting = false
        lines[i].note = nil
        lines[i].saved = nil
        // Retyped while the delete was on its way: it is a new note now.
        return !lines[i].isBlank && lines[i].key != editingKey ? persist(i) : []
    }

    /// Matches the old lines to `paragraphs`: unchanged lines at both ends keep their identity,
    /// and the changed stretch between them is paired up in order, with extra paragraphs becoming
    /// new lines and extra lines deleted. One edit changes one stretch, so a line keeps its
    /// identity (and its note) while it is typed in, split or joined.
    private mutating func align(to paragraphs: [String]) -> [CoreCommand] {
        let old = lines.map(\.text)
        var head = 0
        while head < old.count, head < paragraphs.count, old[head] == paragraphs[head] { head += 1 }
        var tail = 0
        while tail < old.count - head, tail < paragraphs.count - head,
              old[old.count - 1 - tail] == paragraphs[paragraphs.count - 1 - tail] { tail += 1 }
        let oldMiddle = Array(lines[head..<(old.count - tail)])
        let newMiddle = Array(paragraphs[head..<(paragraphs.count - tail)])
        var middle: [LiveNoteLine] = []
        var commands: [CoreCommand] = []
        for (i, text) in newMiddle.enumerated() {
            if i < oldMiddle.count {
                var line = oldMiddle[i]
                line.text = text
                middle.append(line)
            } else {
                middle.append(LiveNoteLine(key: nextKey, text: text))
                nextKey += 1
            }
        }
        for line in oldMiddle.dropFirst(newMiddle.count) {
            guard let note = line.note else {
                // A line whose add is on its way is deleted when note.added names it (added(ref:)).
                continue
            }
            orphans[line.key] = note
            if !line.deleting {
                commands.append(.noteDelete(note: note, ref: ref(line.key, .delete)))
            }
        }
        lines = Array(lines[..<head]) + middle + Array(lines[(old.count - tail)...])
        return commands
    }

    /// What saving line `i` takes now.
    private mutating func persist(_ i: Int) -> [CoreCommand] {
        let line = lines[i]
        if line.deleting {
            return []
        }
        if line.isBlank {
            guard let note = line.note else { return [] }
            lines[i].deleting = true
            lines[i].saved = nil
            return [.noteDelete(note: note, ref: ref(line.key, .delete))]
        }
        guard line.saved != line.text else { return [] }
        if let note = line.note {
            lines[i].saved = line.text
            return [.noteUpdate(note: note, text: line.text, ref: ref(line.key, .update))]
        }
        guard !line.adding else { return [] }
        lines[i].adding = true
        lines[i].saved = line.text
        return [.noteAdd(record: record, atMs: line.atMs ?? 0, text: line.text, ref: ref(line.key))]
    }
}

// MARK: - The question stack and Ask

/// A question the far end asked.
struct StackedQuestion: Equatable, Identifiable {
    let id: Int
    let text: String
    let atMs: Int64
}

/// The far end's questions, newest first, at most four.
struct QuestionStack: Equatable {
    static let capacity = 4
    /// Shorter questions ("Right?") are not worth a slot.
    static let minimumLength = 12

    private(set) var questions: [StackedQuestion] = []
    private var seen: Set<String> = []
    private var nextID = 0

    /// A final from the far end: its questions go on the stack. The user's own questions do not:
    /// what is asked of them is what they may want help with.
    mutating func heard(_ final: MeetingFinal) {
        guard final.channel == .far else { return }
        for sentence in Self.sentences(final.text) where sentence.hasSuffix("?") {
            guard sentence.count - 1 >= Self.minimumLength else { continue }
            let key = sentence.lowercased()
            guard seen.insert(key).inserted else { continue }
            questions.insert(StackedQuestion(id: nextID, text: sentence, atMs: final.startMs), at: 0)
            nextID += 1
        }
        if questions.count > Self.capacity {
            questions.removeLast(questions.count - Self.capacity)
        }
    }

    /// Sentences, each with its end mark.
    static func sentences(_ text: String) -> [String] {
        var out: [String] = []
        var current = ""
        for ch in text {
            current.append(ch)
            if ch == "?" || ch == "." || ch == "!" {
                let s = current.trimmingCharacters(in: .whitespaces)
                if !s.isEmpty { out.append(s) }
                current = ""
            }
        }
        let rest = current.trimmingCharacters(in: .whitespaces)
        if !rest.isEmpty { out.append(rest) }
        return out
    }
}

/// What Ask answered.
enum AskAnswer: Equatable, Sendable {
    case answer(String)
    /// Nothing answered; why, in the user's words.
    case unavailable(String)

    /// What the core's failure means for the user. Its message names what failed (never the
    /// question); the cases without a model, without the user's OK, and after the meeting get their
    /// own words.
    static func failed(_ message: String) -> AskAnswer {
        // The core's NEEDS_CONSENT (ink-ffi asking.rs): nothing was sent. Its start is stable.
        if message.hasPrefix("Ask needs your OK") {
            return .unavailable("Ask needs your OK first: turn on Summaries and Ask in Settings > AI.")
        }
        if message.contains("no language model") {
            return .unavailable("Answers need Apple Intelligence, which is off or not ready on this Mac.")
        }
        if message.contains("no meeting") {
            return .unavailable("Couldn't answer: the meeting has ended.")
        }
        return .unavailable("Couldn't answer that. Try asking again.")
    }
}

/// A question the user asked, and its answer.
struct AskedQuestion: Equatable, Identifiable {
    let id: Int
    let question: String
    /// nil while waiting.
    var answer: AskAnswer?
}

// MARK: - The model

@MainActor
@Observable
final class LiveModel {
    /// The live meeting's record, and when this shell heard it start.
    private(set) var record: String?
    private(set) var startedAt: Date?
    private(set) var stack = QuestionStack()
    private(set) var asked: [AskedQuestion] = []
    var askText = ""
    /// The notes' text, as the editor shows it: restored when the screen comes back.
    private(set) var notesText = ""

    @ObservationIgnored private var draft: LiveNotesDraft?
    /// Whether a record is being stopped and deleted (MeetingModel's): its notes are never saved.
    @ObservationIgnored var discarding: (String) -> Bool = { _ in false }
    /// The meeting stopped while it was being deleted: its notes wait, unsaved, for the delete's
    /// outcome (gone with it, or saved if the core refused the delete).
    @ObservationIgnored private var flushHeld = false
    @ObservationIgnored private var nextAsk = 0
    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let now: () -> Date

    init(send: @escaping SendCommand, now: @escaping () -> Date = Date.init) {
        self.send = send
        self.now = now
    }

    /// Where the meeting is now, ms since it started.
    func elapsedMs(at date: Date? = nil) -> Int64 {
        guard let startedAt else { return 0 }
        return Int64(max(0, (date ?? now()).timeIntervalSince(startedAt)) * 1_000)
    }

    /// The notes after an edit: the editor's whole text and the caret's paragraph.
    func notesEdited(_ text: String, caretParagraph: Int?) {
        notesText = text
        guard var draft else { return }
        let commands = draft.edited(Self.paragraphs(text), caret: caretParagraph, nowMs: UInt64(elapsedMs()))
        self.draft = draft
        commands.forEach(send)
    }

    /// The editor lost focus: save what is typed.
    func notesLeft() {
        guard var draft else { return }
        let commands = draft.flush()
        self.draft = draft
        commands.forEach(send)
    }

    static func paragraphs(_ text: String) -> [String] {
        text.components(separatedBy: "\n")
    }

    /// Asks `askText`, about the meeting so far (`context`, the ledger's settled lines).
    func submitAsk(context: [LiveLine]) {
        let question = askText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !question.isEmpty else { return }
        askText = ""
        send(question: question, context: context)
    }

    /// Answers the stack's question in `slot` (⌘1 is the newest).
    func answerStacked(_ slot: Int, context: [LiveLine]) {
        guard stack.questions.indices.contains(slot) else { return }
        send(question: stack.questions[slot].text, context: context)
    }

    /// Asks the core. `context` (the settled lines on screen) is not sent: the core answers from
    /// the record, which holds every final, not only the ones the ledger keeps in memory.
    private func send(question: String, context: [LiveLine]) {
        let id = nextAsk
        nextAsk += 1
        asked.insert(AskedQuestion(id: id, question: question), at: 0)
        send(.meetingAsk(question: question, ref: Self.askRef(id)))
    }

    static func askRef(_ id: Int) -> String { "ask:\(id)" }

    private func answered(ref: String?, _ answer: AskAnswer) {
        guard let ref, let i = asked.firstIndex(where: { Self.askRef($0.id) == ref }) else { return }
        asked[i].answer = answer
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .meetingStarted(let started):
            record = started.record
            startedAt = now()
            stack = QuestionStack()
            asked = []
            notesText = ""
            flushHeld = false
            draft = LiveNotesDraft(record: started.record)
        case .meetingFinal(let final) where final.record == record:
            stack.heard(final)
        case .meetingAnswered(let answered):
            // Model text: shown as words only (the Ask panel renders it as plain Text).
            self.answered(ref: answered.ref, .answer(answered.text))
        case .commandFailed(let failed) where failed.command == "meeting.ask":
            answered(ref: failed.id, .failed(failed.message))
        case .meetingStopped(let stopped) where stopped.record == record:
            // Capture ended: whatever is typed is saved now, unless the meeting is being deleted
            // (Stop and delete): its words never go into a record that is going.
            if discarding(stopped.record) {
                flushHeld = true
            } else {
                notesLeft()
            }
        case .commandFailed(let failed) where failed.command == "meeting.discard" && flushHeld:
            // The delete was refused after the stop: the meeting is kept, and so are its notes.
            flushHeld = false
            notesLeft()
        case .noteAdded(let added) where added.record == record:
            guard let ref = added.ref else { return }
            update { $0.added(ref: ref, note: added.note) }
        case .noteDeleted(let deleted):
            guard let ref = deleted.ref else { return }
            update { $0.deleted(ref: ref) }
        case .commandFailed(let failed) where ["note.add", "note.update", "note.delete"].contains(failed.command):
            guard let ref = failed.id else { return }
            update { $0.failed(ref: ref) }
        case .meetingFinished(let finished) where finished.record == record:
            end()
        case .meetingFailed(let failed) where failed.record == record:
            end()
        case .meetingWorkerFailed(let failed) where failed.record == record:
            end()
        case .meetingDiscarded(let discarded) where discarded.record == record:
            // Stop and delete: the meeting and its notes are gone with it; nothing is saved to it.
            draft = nil
            end()
        case .coreStopped:
            end()
        default:
            break
        }
    }

    /// Changes the draft and sends what the change takes.
    private func update(_ change: (inout LiveNotesDraft) -> [CoreCommand]) {
        guard var draft else { return }
        let commands = change(&draft)
        self.draft = draft
        commands.forEach(send)
    }

    private func end() {
        // A meeting being deleted keeps none of its notes.
        if let record, discarding(record) { draft = nil }
        notesLeft()
        flushHeld = false
        record = nil
        startedAt = nil
        draft = nil
    }
}
