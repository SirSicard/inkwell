// One record as the Record screen reads it, built from the core's `library.record` answer: the
// ledger (who said what, when), the notes-first merged view, the rendered summary, what is owed
// and the audio's chunks. Pure and value-typed: the view model builds it once per answer.
import Foundation
import InkBridge

/// Who said a line.
enum Speaker: Equatable, Hashable, Sendable {
    /// The mic: the user.
    case you
    /// An imported file's one track, which the core keeps on the mic channel: whoever is on it,
    /// not the user.
    case recording
    /// The far end: a name the user gave, a numbered speaker the diarizer found, or the far end
    /// as a whole when labels were not kept.
    case them(String)

    var label: String {
        switch self {
        case .you: "You"
        case .recording: "Speaker"
        case .them(let name): name
        }
    }

    /// The mic channel's: the user's, or an imported file's track.
    var isYou: Bool {
        if case .them = self { return false }
        return true
    }
}

/// One line of the ledger transcript.
struct LedgerLine: Identifiable, Equatable, Sendable {
    /// Its index in the transcript.
    let id: Int
    let speaker: Speaker
    let startMs: Int64
    let endMs: Int64
    let text: String
}

/// One entry of the notes-first merged view.
struct MergedEntry: Identifiable, Equatable, Sendable {
    enum Kind: Equatable, Sendable {
        /// A note the user typed.
        case note
        /// What was said when the note was written (or, without notes, what the record holds).
        case said(Speaker)
        /// A commitment made around then.
        case owed
    }

    let id: String
    let kind: Kind
    let text: String
    /// Where its chip points on the record's timeline.
    let atMs: Int64
}

/// A commitment as the Record screen lists it, with the line it was said in.
struct OwedEntry: Identifiable, Equatable, Sendable {
    let id: String
    let text: String
    let owner: String?
    let due: String?
    let done: Bool
    /// Where it was said, when the core knows.
    let atMs: Int64?
    /// The transcript line holding that moment: its source, shown beside it so a mismatch is
    /// visible at a glance.
    let citedLine: LedgerLine?
}

/// A playable chunk of a record's audio, on its timeline.
struct TimelineChunk: Equatable, Sendable {
    let channel: Channel
    let path: String
    let startMs: Int64
    let frames: Int64
    let sampleRate: Int
    let channels: Int
    let dataOffset: Int

    var durationMs: Int64 {
        sampleRate > 0 ? frames * 1000 / Int64(sampleRate) : 0
    }

    var endMs: Int64 { startMs + durationMs }
}

/// A summary's decision or action with the transcript line it cites (S2.8: the core keeps each
/// item's span with the summary).
struct SummaryCitation: Equatable, Sendable, Identifiable {
    let id: Int
    let kind: SummaryItemKind
    let text: String
    let atMs: Int64
    /// The line at that moment on that side: the item's source, shown beside it.
    let citedLine: LedgerLine?
}

struct RecordDocument: Equatable, Sendable {
    let record: RecordRow
    let ledger: [LedgerLine]
    let merged: [MergedEntry]
    let summary: SummaryDocument?
    let summaryWrittenAt: Int64?
    /// The summary's decisions and actions, each with its cited line, in the summary's order.
    let summaryItems: [SummaryCitation]
    /// The record's own commitments that stand (not folded into another), in the order said.
    let owed: [OwedEntry]
    /// The people on the far end, as named or numbered.
    let people: [String]
    let chunks: [TimelineChunk]
    /// What the player cannot vouch for: the screen shows each (beside the ledger's status and in
    /// the player bar), so an estimated or partial recording never plays with a precise one's
    /// confidence.
    let playbackCaveats: Caveats

    /// What is uncertain about a record's audio.
    struct Caveats: Equatable, Sendable {
        /// The chunks were placed from the earliest one, the meeting's start not being written:
        /// the two sides may be out of step.
        let timelineEstimated: Bool
        /// Chunk files the core left out of the player (unreadable, or of unknown format or place).
        let leftOut: Int

        /// The words the player bar shows, one per line; `waveformPartial` when a chunk could not
        /// be read for the waveform.
        func messages(waveformPartial: Bool) -> [String] {
            var out: [String] = []
            if timelineEstimated { out.append("Timing estimated: the two sides may be out of step.") }
            if leftOut > 0 || waveformPartial { out.append("Part of this recording can't be played.") }
            return out
        }
    }
    /// The transcript is the final pass's (revision 2 or later), not the live one.
    let isFinal: Bool

    /// How far the chips may go: the audio's end, else the last line's.
    var durationMs: Int64 {
        max(chunks.map(\.endMs).max() ?? 0, ledger.map(\.endMs).max() ?? 0)
    }

    init(_ answer: LibraryRecord) {
        record = answer.record
        isFinal = answer.record.revision >= 2
        let names = Dictionary(answer.speakers.map { ($0.speaker, $0.name) }, uniquingKeysWith: { a, _ in a })
        // Unnamed diarized speakers are numbered in the order they first speak.
        var numbered: [String: Int] = [:]
        for segment in answer.segments where segment.channel == .far {
            if let label = segment.speaker, names[label] == nil, numbered[label] == nil {
                numbered[label] = numbered.count + 1
            }
        }
        func speaker(_ segment: RecordSegment) -> Speaker {
            guard segment.channel == .far else { return answer.record.kind == .fileImport ? .recording : .you }
            guard let label = segment.speaker else { return .them("Them") }
            if let name = names[label] { return .them(name) }
            return .them("Speaker \(numbered[label] ?? 1)")
        }
        let ledger = answer.segments.enumerated().map { index, segment in
            LedgerLine(
                id: index, speaker: speaker(segment), startMs: segment.startMs, endMs: segment.endMs,
                text: segment.text.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        self.ledger = ledger

        var people: [String] = []
        for line in ledger {
            if case .them(let name) = line.speaker, !people.contains(name) {
                people.append(name)
            }
        }
        self.people = people

        summary = answer.summary.map { SummaryDocument(markdown: $0.text) }
        summaryWrittenAt = answer.summary?.createdAtUnixMs
        summaryItems = (answer.summary?.items ?? []).enumerated().map { index, item in
            SummaryCitation(
                id: index, kind: item.kind, text: item.text, atMs: item.span.startMs,
                citedLine: Self.line(at: item.span.startMs, in: ledger, channel: item.span.channel))
        }

        let standing = answer.commitments.filter { $0.mergedInto == nil }
        owed = standing.map { c in
            let at = c.provenance.map(\.startMs).min()
            return OwedEntry(
                id: c.commitment, text: c.text, owner: c.owner, due: c.due, done: c.done, atMs: at,
                citedLine: at.flatMap { Self.line(at: $0, in: ledger, channel: c.provenance.first?.channel) })
        }

        merged = Self.merge(
            notes: answer.notes.sorted { ($0.atMs, $0.note) < ($1.atMs, $1.note) }, ledger: ledger, owed: owed)

        chunks = (answer.audio?.chunks ?? []).map {
            TimelineChunk(
                channel: $0.channel, path: $0.path, startMs: $0.startMs, frames: $0.frames,
                sampleRate: Int($0.sampleRate), channels: Int($0.channels), dataOffset: Int($0.dataOffset))
        }
        playbackCaveats = Caveats(
            timelineEstimated: answer.audio?.timeline == .estimated,
            leftOut: Int(answer.audio?.leftOut ?? 0))
    }

    /// The ledger's status: blotted (the final pass's transcript) or live, and whether the audio's
    /// timing is estimated. `blottedAt` is the summary's time, formatted.
    func ledgerStatus(blottedAt: String?) -> String {
        var parts: [String]
        if isFinal {
            parts = [blottedAt.map { "Blotted \($0)" } ?? "Blotted", "final"]
        } else {
            parts = ["Live transcript", "not blotted"]
        }
        if playbackCaveats.timelineEstimated { parts.append("timing estimated") }
        return parts.joined(separator: " · ")
    }

    /// The ledger line to highlight while the playhead is at `ms`: the last line that has started.
    func line(atPlayhead ms: Int64) -> LedgerLine? {
        ledger.last(where: { $0.startMs <= ms })
    }

    /// The line holding `ms` on `channel` (or either side), else the nearest one before it.
    static func line(at ms: Int64, in ledger: [LedgerLine], channel: Channel?) -> LedgerLine? {
        let side = ledger.filter { line in
            guard let channel else { return true }
            return (channel == .mic) == line.speaker.isYou
        }
        // Where one line ends as the next starts, the one starting there.
        return side.last(where: { $0.startMs <= ms && ms <= $0.endMs })
            ?? side.last(where: { $0.startMs <= ms })
            ?? side.first
    }

    /// Notes first: each note, then what was said as it was written (the first lines from a few
    /// seconds before it), then what was promised before the next note. Without notes, the
    /// record's commitments lead, each with its moment.
    static func merge(notes: [RecordNote], ledger: [LedgerLine], owed: [OwedEntry]) -> [MergedEntry] {
        var out: [MergedEntry] = []
        var placed = Set<String>()
        /// Lines from this long before a note count as "what was being said".
        let lead: Int64 = 5_000
        for (i, note) in notes.enumerated() {
            let next = i + 1 < notes.count ? notes[i + 1].atMs : Int64.max
            out.append(MergedEntry(id: "note-\(note.note)", kind: .note, text: note.text, atMs: note.atMs))
            let said = ledger.filter { $0.startMs >= note.atMs - lead && $0.startMs < next }.prefix(2)
            for line in said {
                out.append(MergedEntry(
                    id: "said-\(note.note)-\(line.id)", kind: .said(line.speaker), text: line.text, atMs: line.startMs))
            }
            for item in owed where !placed.contains(item.id) {
                guard let at = item.atMs, at >= note.atMs - lead, at < next else { continue }
                placed.insert(item.id)
                out.append(MergedEntry(id: "owed-\(item.id)", kind: .owed, text: item.text, atMs: at))
            }
        }
        // Commitments no note sits near (all of them, without notes).
        for item in owed where !placed.contains(item.id) {
            guard let at = item.atMs else { continue }
            out.append(MergedEntry(id: "owed-\(item.id)", kind: .owed, text: item.text, atMs: at))
        }
        return out
    }
}
