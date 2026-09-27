// The Library and Record screens' logic: the one record order, how records read, the rendered
// summary (no markdown token survives), the record document (ledger, notes-first merge, what is
// owed and where it was said), and the view model's questions and answers.
import Foundation
import InkBridge
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    do {
        return try InkEvent.decode(Data(json.utf8))
    } catch {
        XCTFail("not an event: \(error)\n\(json)", file: file, line: line)
        return .unknown(type: "")
    }
}

private func row(_ id: String, _ kind: String = "meeting", start: Int64, end: Int64? = nil, title: String? = nil) -> String {
    var fields = [#""record":"\#(id)""#, #""kind":"\#(kind)""#, #""started_at_unix_ms":\#(start)"#, #""revision":2"#, #""has_audio":false"#]
    if let end { fields.append(#""ended_at_unix_ms":\#(end)"#) }
    if let title { fields.append(#""title":"\#(title)""#) }
    return "{" + fields.joined(separator: ",") + "}"
}

private func rows(_ answer: String) -> [RecordRow] {
    guard case .libraryRecords(let records) = event(answer) else { return [] }
    return records.records
}

/// The id a command carried.
private func requestID(_ json: String) -> String {
    let object = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any]
    return object?["id"] as? String ?? ""
}

private func command(_ json: String) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
}

/// The summary the meeting chain stored for the seeded launch meeting (ink-llm's render_markdown
/// over a summary answer): what the screen must render without a token of markdown.
let seededSummary = """
Launch moves to the 14th, after the design review

### Launch date

The design review needs **another week**, so the launch moves to the *fourteenth*.

### Beta

- Keep the beta group small, about forty people
- Security wants it in writing that the models run on the laptop

1. Revised plan first
2. Then the budget sheet

## Decisions
- Move the launch to the fourteenth
- Keep the beta group to about forty

## Actions
- Send the revised plan and the budget sheet (You, Friday)

## Open questions
- Who signs off the design review?
"""

// MARK: - Order

/// The step's sort-order test, on the shell's side: whatever order records arrive in, they are
/// shown newest first by start, ties by id; "the last meeting" is the latest *finished* one.
final class RecordOrderTests: XCTestCase {
    func testRecordsShowNewestFirstByStartWhateverOrderTheyArrive() {
        let shuffled = rows(#"""
        {"type":"library.records","more":false,"records":[
          \#(row("b", start: 2_000, end: 3_000)), \#(row("d", start: 9_000, end: 9_500)),
          \#(row("a", start: 1_000, end: 1_500)), \#(row("c2", start: 5_000, end: 6_000)),
          \#(row("c1", start: 5_000, end: 6_000)), \#(row("e", "dictation", start: 7_000, end: 7_100))]}
        """#)
        XCTAssertEqual(RecordOrder.newestFirst(shuffled).map(\.record), ["d", "e", "c2", "c1", "b", "a"])
    }

    func testTheLastMeetingIsTheLatestFinishedOneNotTheLatestWritten() {
        let arrived = rows(#"""
        {"type":"library.records","more":false,"records":[
          \#(row("older", start: 1_000, end: 2_000)), \#(row("live", start: 9_000)),
          \#(row("newest", start: 5_000, end: 6_000)), \#(row("dictated", "dictation", start: 8_000, end: 8_100))]}
        """#)
        XCTAssertEqual(RecordOrder.latestFinishedMeeting(arrived)?.record, "newest")
    }

    func testAPageMergesIntoTheListInOrderWithoutRepeats() {
        let first = rows(#"{"type":"library.records","more":true,"records":[\#(row("x", start: 9)), \#(row("y", start: 7))]}"#)
        let second = rows(#"{"type":"library.records","more":false,"records":[\#(row("y", start: 7)), \#(row("z", start: 8))]}"#)
        XCTAssertEqual(RecordOrder.newestFirst(first + second).map(\.record), ["x", "z", "y"])
    }

    @MainActor
    func testTheModelShowsAnAnswerSortedAndKeepsItSortedAcrossPages() {
        var sent: [String] = []
        let library = LibraryModel(send: { sent.append($0.json) })
        library.refreshList()
        let id = requestID(sent.last!)
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(id)","more":true,"kind":"meeting","records":[
          \#(row("m1", start: 1_000, end: 2_000)), \#(row("m3", start: 3_000, end: 4_000)), \#(row("m2", start: 2_000, end: 3_000))]}
        """#)])
        XCTAssertEqual(library.records.map(\.record), ["m3", "m2", "m1"])
        library.loadMore()
        let more = command(sent.last!)
        XCTAssertEqual(more["cmd"] as? String, "records.list")
        XCTAssertEqual((more["before"] as? [String: Any])?["id"] as? String, "m1", "the cursor is the last shown")
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(more["id"] as! String)","more":false,"kind":"meeting","records":[\#(row("m0", start: 500, end: 900))]}
        """#)])
        XCTAssertEqual(library.records.map(\.record), ["m3", "m2", "m1", "m0"])
        XCTAssertFalse(library.hasMore)
    }
}

// MARK: - Format

final class LibraryFormatTests: XCTestCase {
    private let calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "Europe/Madrid")!
        c.locale = Locale(identifier: "en_GB")
        return c
    }()

    /// Thursday 24 September 2026, 15:00 in Madrid.
    private let now = Date(timeIntervalSince1970: 1_790_254_800)

    func testStampsAndLengths() {
        XCTAssertEqual(LibraryFormat.stamp(ms: 761_000), "12:41")
        XCTAssertEqual(LibraryFormat.stamp(ms: 3_725_000), "1:02:05")
        XCTAssertEqual(LibraryFormat.stamp(ms: -5), "00:00")
        XCTAssertEqual(LibraryFormat.duration(ms: 20_000), "under 1 min")
        XCTAssertEqual(LibraryFormat.duration(ms: 42 * 60_000), "42 min")
        XCTAssertEqual(LibraryFormat.duration(ms: 130 * 60_000), "2 h 10 min")
    }

    func testDaysReadAsToday_Yesterday_OrTheDate() {
        XCTAssertEqual(LibraryFormat.day(now.addingTimeInterval(-3_600), now: now, calendar: calendar), "Today")
        XCTAssertEqual(LibraryFormat.day(now.addingTimeInterval(-86_400), now: now, calendar: calendar), "Yesterday")
        // Older days read as the locale's short date; ICU versions spell the month differently
        // ("Sep", "Sept"), so only its shape is pinned.
        let older = LibraryFormat.day(now.addingTimeInterval(-3 * 86_400), now: now, calendar: calendar)
        XCTAssertTrue(older.hasPrefix("Mon 21 Sep"), older)
        XCTAssertEqual(LibraryFormat.time(now, calendar: calendar), "15:00")
        XCTAssertEqual(LibraryFormat.greeting(now, calendar: calendar), "Good afternoon")
    }

    func testARecordIsCalledByItsTitleThenItsWordsNeverUntitled() {
        let titled = rows(#"{"type":"library.records","more":false,"records":[\#(row("a", start: 0, title: "Launch moves to the 14th"))]}"#)[0]
        XCTAssertEqual(LibraryFormat.title(of: titled), "Launch moves to the 14th")
        let dictated = rows(#"""
        {"type":"library.records","more":false,"records":[{"record":"d","kind":"dictation","started_at_unix_ms":0,"revision":1,"has_audio":false,"preview":"Remind me to book the room"}]}
        """#)[0]
        XCTAssertEqual(LibraryFormat.title(of: dictated), "Remind me to book the room")
        let bare = rows(#"{"type":"library.records","more":false,"records":[\#(row("m", start: 0, title: "  "))]}"#)[0]
        XCTAssertEqual(LibraryFormat.title(of: bare), "Meeting")
    }

    func testListAndHeaderLines() {
        let start = Int64(now.timeIntervalSince1970 * 1000) - 60 * 60_000
        let meeting = rows(#"""
        {"type":"library.records","more":false,"records":[{"record":"m","kind":"meeting","started_at_unix_ms":\#(start),"ended_at_unix_ms":\#(start + 42 * 60_000),"source_app":"Zoom","revision":2,"has_audio":true}]}
        """#)[0]
        XCTAssertEqual(LibraryFormat.listLine(meeting, now: now, calendar: calendar), "Today · 14:00 · 42 min")
        XCTAssertEqual(
            LibraryFormat.headerLine(meeting, people: ["You", "Alex", "Robin"], now: now, calendar: calendar),
            "Today 14:00 · 42 min · Zoom · You, Alex, Robin")
    }
}

// MARK: - Summary

/// The step's snapshot check: the rendered summary contains no markdown tokens, and what was
/// marked up is styled, not lost.
final class SummaryRenderingTests: XCTestCase {
    /// Markdown syntax that must never reach the screen.
    private func assertNoMarkdown(_ text: String, file: StaticString = #filePath, line: UInt = #line) {
        for token in ["**", "__", "`", "~~", "](", "[x]", "[ ]"] {
            XCTAssertFalse(text.contains(token), "“\(token)” in:\n\(text)", file: file, line: line)
        }
        for l in text.components(separatedBy: "\n") {
            let t = l.trimmingCharacters(in: .whitespaces)
            XCTAssertFalse(t.hasPrefix("#"), "a heading marker in: \(l)", file: file, line: line)
            XCTAssertFalse(t.hasPrefix("- ") || t.hasPrefix("* ") || t.hasPrefix("+ "), "a list marker in: \(l)", file: file, line: line)
            XCTAssertFalse(t.hasPrefix(">"), "a quote marker in: \(l)", file: file, line: line)
            XCTAssertFalse(t == "---" || t == "***", "a rule in: \(l)", file: file, line: line)
        }
        XCTAssertNil(text.range(of: #"(?<!\w)\*\S[^*]*\*(?!\w)"#, options: .regularExpression), "italic stars in:\n\(text)", file: file, line: line)
    }

    func testTheStoredSummaryRendersWithoutMarkdownTokens() {
        let doc = SummaryDocument(markdown: seededSummary)
        let snapshot = doc.plainText
        assertNoMarkdown(snapshot)
        XCTAssertEqual(doc.headline.map { String($0.characters) }, "Launch moves to the 14th, after the design review")
        XCTAssertTrue(snapshot.contains("The design review needs another week, so the launch moves to the fourteenth."), snapshot)
        XCTAssertTrue(snapshot.contains("• Keep the beta group small, about forty people"))
        XCTAssertTrue(snapshot.contains("1. Revised plan first"))
        XCTAssertTrue(snapshot.contains("\nDecisions\n"), "a section heading is its words")
        let headings = doc.blocks.compactMap { block -> String? in
            if case .heading(_, let text) = block { return String(text.characters) }
            return nil
        }
        XCTAssertEqual(headings, ["Launch date", "Beta", "Decisions", "Actions", "Open questions"])
    }

    func testEmphasisIsStyledNotDropped() throws {
        let doc = SummaryDocument(markdown: seededSummary)
        guard case .paragraph(let body)? = doc.blocks.first(where: {
            if case .paragraph = $0 { true } else { false }
        }) else { return XCTFail("no paragraph") }
        let bold = body.runs.filter { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true }
        XCTAssertEqual(bold.map { String(body[$0.range].characters) }, ["another week"])
        let italic = body.runs.filter { $0.inlinePresentationIntent?.contains(.emphasized) == true }
        XCTAssertEqual(italic.map { String(body[$0.range].characters) }, ["fourteenth"])
    }

    func testAnythingAModelMightWriteLosesItsSyntax() {
        let messy = """
        # Weekly sync #
        Status: **green** and __steady__, see [the plan](https://example.com/plan) and `build 42`.
        > Quoted from the call: ~~never~~ always ship on Friday.

        ---
        * [ ] draft the notes
        + [x] book the room
        10) numbered with a paren
        ```
        code stays as its words
        ```
        A **broken bold at the end
        """
        let snapshot = SummaryDocument(markdown: messy).plainText
        assertNoMarkdown(snapshot)
        for words in ["Weekly sync", "green and steady", "the plan", "build 42", "always ship on Friday",
                      "draft the notes", "book the room", "10. numbered with a paren", "code stays as its words",
                      "A broken bold at the end"] {
            XCTAssertTrue(snapshot.contains(words), "“\(words)” missing from:\n\(snapshot)")
        }
        XCTAssertFalse(snapshot.contains("example.com"), "a link shows its text, not its address")
    }
}

// MARK: - Record document

private let recordAnswer = #"""
{"type":"library.record","ref":"REQ",
 "record":{"record":"r1","kind":"meeting","title":"Launch moves to the 14th","started_at_unix_ms":1790250000000,"ended_at_unix_ms":1790250030000,"source_app":"Zoom","revision":2,"has_audio":true},
 "segments":[
   {"channel":"far","start_ms":0,"end_ms":10000,"text":"The design review needs another week.","speaker":"spk0"},
   {"channel":"mic","start_ms":486,"end_ms":1274,"text":"Let's settle the launch date."},
   {"channel":"mic","start_ms":2822,"end_ms":3546,"text":"I'll send the revised plan by Friday."},
   {"channel":"far","start_ms":10000,"end_ms":13466,"text":"Can you share the budget?","speaker":"spk1"},
   {"channel":"far","start_ms":18278,"end_ms":19258,"text":"Agreed, the fourteenth works.","speaker":"spk2"},
   {"channel":"far","start_ms":20000,"end_ms":21000,"text":"Thanks, all."}],
 "notes":[{"note":"n2","at_ms":15000,"text":"Beta: small group"},{"note":"n1","at_ms":2000,"text":"Launch date"}],
 "summary":{"text":"Launch moves to the 14th\n\nThe review needs **another week**.","model":"scripted/seed","created_at_unix_ms":1790250031000},
 "commitments":[
   {"commitment":"c1","record":"r1","text":"Send the revised plan","owner":"You","due":"Friday","provenance":[{"channel":"mic","start_ms":2822,"end_ms":3546}],"merged_into":"c2","done":false},
   {"commitment":"c2","record":"r1","text":"Send the revised plan","due":"Friday","provenance":[{"channel":"mic","start_ms":2822,"end_ms":3546}],"done":false},
   {"commitment":"c3","record":"r1","text":"Share the budget","provenance":[{"channel":"far","start_ms":10000,"end_ms":13466}],"done":true}],
 "speakers":[{"speaker":"spk0","name":"Alex"}],
 "audio":{"timeline":"recorded","left_out":0,"chunks":[
   {"channel":"mic","path":"/nonexistent/mic-000000-16000x1.pcm","start_ms":3,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64},
   {"channel":"far","path":"/nonexistent/far-000000-16000x1.pcm","start_ms":3,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
"""#

private func document(_ request: String = "REQ") -> RecordDocument? {
    guard case .libraryRecord(let answer) = event(recordAnswer.replacingOccurrences(of: "REQ", with: request)) else { return nil }
    return RecordDocument(answer)
}

final class RecordDocumentTests: XCTestCase {
    func testTheLedgerNamesEachSpeakerByStreamAndName() throws {
        let doc = try XCTUnwrap(document())
        XCTAssertEqual(doc.ledger.map(\.speaker.label), ["Alex", "You", "You", "Speaker 1", "Speaker 2", "Them"])
        XCTAssertEqual(doc.people, ["Alex", "Speaker 1", "Speaker 2", "Them"])
        XCTAssertTrue(doc.isFinal)
        XCTAssertEqual(doc.durationMs, 30_003, "the audio's end")
        XCTAssertEqual(doc.line(atPlayhead: 11_000)?.text, "Can you share the budget?")
        XCTAssertNil(doc.line(atPlayhead: -1))
    }

    func testNotesComeFirstEachFilledInWithWhatWasSaidAndPromisedThen() throws {
        let doc = try XCTUnwrap(document())
        let merged = doc.merged.map { entry -> String in
            switch entry.kind {
            case .note: "note \(entry.text) @\(entry.atMs)"
            case .said(let who): "said \(who.label) @\(entry.atMs)"
            case .owed: "owed \(entry.text) @\(entry.atMs)"
            }
        }
        XCTAssertEqual(merged, [
            "note Launch date @2000",
            "said Alex @0", "said You @486",
            "owed Send the revised plan @2822", "owed Share the budget @10000",
            "note Beta: small group @15000",
            "said Speaker 1 @10000", "said Speaker 2 @18278",
        ])
    }

    func testOwedListsWhatStandsWithTheLineItWasSaidIn() throws {
        let doc = try XCTUnwrap(document())
        XCTAssertEqual(doc.owed.map(\.id), ["c2", "c3"], "the merged duplicate is folded away")
        XCTAssertEqual(doc.owed[0].citedLine?.text, "I'll send the revised plan by Friday.")
        XCTAssertEqual(doc.owed[1].citedLine?.speaker, .them("Speaker 1"))
        XCTAssertTrue(doc.owed[1].done)
        XCTAssertEqual(doc.summary?.headline.map { String($0.characters) }, "Launch moves to the 14th")
        XCTAssertEqual(doc.chunks.count, 2)
    }
}

// MARK: - The view model

@MainActor
final class LibraryModelTests: XCTestCase {
    private func model() -> (LibraryModel, () -> [String]) {
        var sent: [String] = []
        let library = LibraryModel(send: { sent.append($0.json) })
        library.makePlayer = { _ in nil }
        return (library, { sent })
    }

    /// Every library command carries an id, and the answer (or its failure) is matched by it.
    func testCommandsCarryTheirFieldsAndAnId() {
        let list = command(CoreCommand.recordsList(
            kind: .fileImport, before: .init(startedAtUnixMs: 7, record: "r"), limit: 5, ref: "q1").json)
        XCTAssertEqual(list["cmd"] as? String, "records.list")
        XCTAssertEqual(list["kind"] as? String, "file_import")
        XCTAssertEqual(list["id"] as? String, "q1")
        XCTAssertEqual((list["before"] as? [String: Any])?["started_at_unix_ms"] as? Int, 7)
        XCTAssertNil(command(CoreCommand.recordsList(kind: nil, before: nil, limit: 5, ref: "q2").json)["kind"])
        for c in [CoreCommand.recordsSearch(query: "x", limit: 1, ref: "a"), .recordOpen(record: "r", ref: "b"),
                  .libraryStats(sinceUnixMs: 0, ref: "c")] {
            XCTAssertNotNil(command(c.json)["id"] as? String, c.name)
        }
    }

    func testAnAnswerToAnOlderQuestionIsDropped() {
        let (library, sent) = model()
        library.query = "bud"
        let first = requestID(sent().last!)
        library.query = "budget"
        let second = requestID(sent().last!)
        library.apply([event(#"{"type":"library.search","ref":"\#(second)","query":"budget","hits":[{"record":"r","started_at_unix_ms":0,"start_ms":5,"snippet":"the budget"}]}"#)])
        library.apply([event(#"{"type":"library.search","ref":"\#(first)","query":"bud","hits":[]}"#)])
        XCTAssertEqual(library.hits.map(\.snippet), ["the budget"], "the stale answer did not replace the newer one")
        library.query = "  "
        XCTAssertTrue(library.hits.isEmpty, "an empty query clears the matches")
    }

    func testOpeningARecordShowsItAndAMissingOneSaysSo() throws {
        let (library, sent) = model()
        library.open("r1")
        XCTAssertEqual(library.selected, "r1")
        XCTAssertNil(library.document)
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        XCTAssertEqual(library.document?.record.record, "r1")

        library.open("gone")
        let id = requestID(sent().last!)
        library.apply([event(#"{"type":"command.failed","command":"record.open","id":"\#(id)","message":"there is no record gone"}"#)])
        XCTAssertNil(library.document)
        XCTAssertEqual(library.openFailure, "there is no record gone")
    }

    func testTodayAsksForTheLatestFinishedMeetingThenOpensIt() throws {
        let (library, sent) = model()
        library.refreshToday()
        let asked = sent().map(command)
        XCTAssertEqual(asked.map { $0["cmd"] as? String }, ["records.list", "library.stats", "library.stats"],
                       "what is owed is the Owed model's")
        let listID = asked[0]["id"] as! String
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(listID)","more":false,"kind":"meeting","records":[
          \#(row("live", start: 9_000)), \#(row("old", start: 1_000, end: 2_000)), \#(row("r1", start: 5_000, end: 6_000))]}
        """#)])
        let open = command(sent().last!)
        XCTAssertEqual(open["cmd"] as? String, "record.open")
        XCTAssertEqual(open["record"] as? String, "r1")
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: open["id"] as! String))])
        XCTAssertEqual(library.lastMeeting?.record.record, "r1")
        XCTAssertNil(library.document, "Today's record is not the Library's selection")
    }

    func testANewRecordOrAMarkedCommitmentRefreshesWhatIsShown() {
        let (library, sent) = model()
        let before = sent().count
        library.apply([event(#"{"type":"meeting.finished","record":"r9","revision":2}"#)])
        let asked = sent()[before...].map { command($0)["cmd"] as? String }
        XCTAssertTrue(asked.contains("records.list") && asked.contains("library.stats"), "\(asked)")
        library.setDone("c1", true)
        XCTAssertEqual(command(sent().last!)["cmd"] as? String, "commitment.set_done")

        // A commitment changed while a record is open: the record is read again.
        library.open("r1")
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        let reads = sent().count
        library.apply([event(#"{"type":"commitment.updated","commitment":"c2","done":true}"#)])
        XCTAssertEqual(sent().count, reads + 1)
        XCTAssertEqual(command(sent().last!)["cmd"] as? String, "record.open")
    }

    /// A list or a record that could not be read says so; it never reads as an empty library, and
    /// a stale question's failure changes nothing.
    func testAFailedLoadReadsAsCouldNotLoadNeverAsEmpty() {
        let (library, sent) = model()
        library.refreshList()
        let first = requestID(sent().last!)
        XCTAssertEqual(library.listLoad, .loading)
        library.refreshList()
        let second = requestID(sent().last!)
        let failed = { (id: String, command: String) in
            event(#"{"type":"command.failed","command":"\#(command)","id":"\#(id)","message":"the library: disk I/O error"}"#)
        }
        library.apply([failed(first, "records.list")])
        XCTAssertEqual(library.listLoad, .loading, "a stale failure changes nothing")
        library.apply([failed(second, "records.list")])
        XCTAssertEqual(library.listLoad, .failed)
        XCTAssertTrue(library.records.isEmpty)

        library.refreshToday()
        let today = sent().map(command).last { $0["cmd"] as? String == "records.list" }?["id"] as? String ?? ""
        library.apply([failed(today, "records.list")])
        XCTAssertEqual(library.lastMeetingLoad, .failed, "not \"no meetings yet\"")

        library.query = "budget"
        library.apply([failed(requestID(sent().last!), "records.search")])
        XCTAssertEqual(library.searchLoad, .failed, "not \"nothing matches\"")

        // The screens show these; the controller logs the rest by name.
        guard case .commandFailed(let listFailure) = failed(second, "records.list"),
              case .commandFailed(let stats) = failed("statsDay-99", "library.stats"),
              case .commandFailed(let other) = failed("setting:x", "setting.get")
        else { return XCTFail("not failures") }
        XCTAssertTrue(library.handles(listFailure))
        XCTAssertFalse(library.handles(stats), "counts that cannot be read are left out, and logged")
        XCTAssertFalse(library.handles(other))
    }
}
