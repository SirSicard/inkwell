// The one order records are shown in: newest first by when they started, then by id. The core
// lists them that way; the shell keeps it when pages merge or a record changes, and never sorts
// by anything else (the earlier app's "latest record" was the latest written, not the latest
// held).
import InkBridge

enum RecordOrder {
    /// Whether `a` comes before `b`: it started later, or at the same moment with the larger id.
    static func precedes(_ a: RecordRow, _ b: RecordRow) -> Bool {
        if a.startedAtUnixMs != b.startedAtUnixMs {
            return a.startedAtUnixMs > b.startedAtUnixMs
        }
        return a.record > b.record
    }

    /// `rows` newest first, each record once (a later copy replaces an earlier one).
    static func newestFirst(_ rows: [RecordRow]) -> [RecordRow] {
        var byID: [String: RecordRow] = [:]
        for row in rows {
            byID[row.record] = row
        }
        return byID.values.sorted(by: precedes)
    }

    /// The meeting that started last among those that have ended: a meeting still being recorded
    /// is not "the last meeting".
    static func latestFinishedMeeting(_ rows: [RecordRow]) -> RecordRow? {
        newestFirst(rows.filter { $0.kind == .meeting && $0.endedAtUnixMs != nil }).first
    }
}
