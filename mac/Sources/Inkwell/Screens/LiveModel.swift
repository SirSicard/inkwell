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
//   so one can be answered with a key press, and anything else can be asked.
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

    /// Everything the core has for the live meeting, finals first, then each side's partial.
    static func ledger(_ meeting: CoreStore.LiveMeeting) -> [LiveLine] {
        var lines = meeting.finals.enumerated().map { index, final in
            LiveLine(id: "final-\(index)", channel: final.channel, atMs: final.startMs, text: final.text, wet: false)
        }
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
    /// The core's id, once added.
    var note: String?
    /// The text the core has (or has been sent).
    var saved: String?
    /// note.add is on its way.
    var adding = false

    var isBlank: Bool { text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
}

/// The notes of one live meeting: paragraphs in, note commands out.
struct LiveNotesDraft {
    private(set) var lines: [LiveNoteLine] = [LiveNoteLine(key: 0, text: "")]
    private var nextKey = 1
    /// The line with the caret; it is saved when the user moves on.
    private var editingKey: Int?
    /// The meeting's record.
    let record: String

    init(record: String) {
        self.record = record
    }

    static func ref(_ key: Int) -> String { "note-line-\(key)" }

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

    /// Saves every line, the one being edited included (focus left, or the meeting ended).
    mutating func flush() -> [CoreCommand] {
        editingKey = nil
        return lines.indices.flatMap { persist($0) }
    }

    /// The core added the note sent for `ref`.
    mutating func added(ref: String, note: String) -> [CoreCommand] {
        guard let i = lines.firstIndex(where: { Self.ref($0.key) == ref }) else {
            // Its line was deleted while the add was on its way.
            return [.noteDelete(note: note)]
        }
        lines[i].note = note
        lines[i].adding = false
        return lines[i].key == editingKey ? [] : persist(i)
    }

    /// The core refused the add sent for `ref`: the line is unsaved and is tried again on the next
    /// save.
    mutating func addFailed(ref: String) {
        if let i = lines.firstIndex(where: { Self.ref($0.key) == ref }) {
            lines[i].adding = false
            lines[i].saved = nil
        }
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
            if let note = line.note {
                commands.append(.noteDelete(note: note))
            }
            // A line whose add is on its way is deleted when note.added names it (added(ref:)).
        }
        lines = Array(lines[..<head]) + middle + Array(lines[(old.count - tail)...])
        return commands
    }

    /// What saving line `i` takes now.
    private mutating func persist(_ i: Int) -> [CoreCommand] {
        let line = lines[i]
        if line.isBlank {
            guard let note = line.note else { return [] }
            lines[i].note = nil
            lines[i].saved = nil
            return [.noteDelete(note: note)]
        }
        guard line.saved != line.text else { return [] }
        if let note = line.note {
            lines[i].saved = line.text
            return [.noteUpdate(note: note, text: line.text)]
        }
        guard !line.adding else { return [] }
        lines[i].adding = true
        lines[i].saved = line.text
        return [.noteAdd(record: record, atMs: line.atMs ?? 0, text: line.text, ref: Self.ref(line.key))]
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
    /// Nothing can answer; why, in the user's words.
    case unavailable(String)
}

/// Answers a question about the meeting so far.
protocol AskService: Sendable {
    func answer(_ question: String, context: [LiveLine]) async -> AskAnswer
}

/// Until the core can answer questions about a meeting, Ask says so rather than guessing.
struct AskNotAvailable: AskService {
    func answer(_ question: String, context: [LiveLine]) async -> AskAnswer {
        .unavailable("Answers about a call aren't available in this version yet.")
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
    @ObservationIgnored private var nextAsk = 0
    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let askService: any AskService
    @ObservationIgnored private let now: () -> Date

    init(send: @escaping SendCommand, ask: any AskService = AskNotAvailable(), now: @escaping () -> Date = Date.init) {
        self.send = send
        askService = ask
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

    private func send(question: String, context: [LiveLine]) {
        let id = nextAsk
        nextAsk += 1
        asked.insert(AskedQuestion(id: id, question: question), at: 0)
        let service = askService
        Task { [weak self] in
            let answer = await service.answer(question, context: context)
            self?.answered(id, answer)
        }
    }

    private func answered(_ id: Int, _ answer: AskAnswer) {
        if let i = asked.firstIndex(where: { $0.id == id }) {
            asked[i].answer = answer
        }
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .meetingStarted(let started):
            record = started.record
            startedAt = now()
            stack = QuestionStack()
            asked = []
            notesText = ""
            draft = LiveNotesDraft(record: started.record)
        case .meetingFinal(let final) where final.record == record:
            stack.heard(final)
        case .meetingStopped(let stopped) where stopped.record == record:
            // Capture ended: whatever is typed is saved now.
            notesLeft()
        case .noteAdded(let added) where added.record == record:
            guard let ref = added.ref, var draft else { return }
            let commands = draft.added(ref: ref, note: added.note)
            self.draft = draft
            commands.forEach(send)
        case .commandFailed(let failed) where failed.command == "note.add":
            if let ref = failed.id { draft?.addFailed(ref: ref) }
        case .meetingFinished(let finished) where finished.record == record:
            end()
        case .meetingFailed(let failed) where failed.record == record:
            end()
        case .meetingWorkerFailed(let failed) where failed.record == record:
            end()
        case .coreStopped:
            end()
        default:
            break
        }
    }

    private func end() {
        notesLeft()
        record = nil
        startedAt = nil
        draft = nil
    }
}
