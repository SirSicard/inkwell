// The Library and Record screens' logic: the one record order, how records read, the rendered
// summary (no markdown token survives), the record document (ledger, notes-first merge, what is
// owed and where it was said), and the view model's questions and answers.
import AppKit
import Foundation
import InkBridge
import SwiftUI
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

// MARK: - Links in summaries

/// Review fix (security): a summary is written by a language model from what the far end said, so
/// a link in it is untrusted. The rendered summary keeps a link's words and drops where it goes:
/// no run anywhere carries a link.
final class SummaryLinkTests: XCTestCase {
    private func links(_ text: AttributedString?) -> [URL] {
        guard let text else { return [] }
        return text.runs.compactMap(\.link)
    }

    private func allRuns(_ doc: SummaryDocument) -> [AttributedString] {
        var out: [AttributedString] = []
        if let headline = doc.headline { out.append(headline) }
        for block in doc.blocks {
            switch block {
            case .heading(_, let t), .paragraph(let t), .item(_, let t): out.append(t)
            }
        }
        return out
    }

    func testNoRenderedRunCarriesALinkAndTheWordsStay() {
        let markdown = """
        Call [the vendor](https://evil.example/pay) today, see <https://evil.example/auto>.

        ## Actions
        - Read [**the brief**](http://evil.example/x "title") first
        - Mail [me](mailto:someone@example.com)
        """
        let doc = SummaryDocument(markdown: markdown)
        for run in allRuns(doc) {
            XCTAssertEqual(links(run), [], String(run.characters))
        }
        let text = doc.plainText
        for words in ["Call the vendor today", "Read the brief first", "Mail me"] {
            XCTAssertTrue(text.contains(words), "“\(words)” missing from:\n\(text)")
        }
        XCTAssertFalse(text.contains("evil.example/pay"), "a hidden destination never shows as text either")
        XCTAssertEqual(links(doc.lede), [])
    }

    func testTheViewsThatShowASummaryRefuseToOpenLinks() {
        XCTAssertEqual(SummaryLinks.decide(URL(string: "https://evil.example")!), .refused)
        XCTAssertEqual(SummaryLinks.decide(URL(string: "mailto:a@b.example")!), .refused)
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
 "summary":{"text":"Launch moves to the 14th\n\nThe review needs **another week**.","model":"scripted/seed","created_at_unix_ms":1790250031000,"items":[]},
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
        XCTAssertEqual(doc.ledger.map(\.speaker.label), ["Alex", "You", "You", "Speaker 2", "Speaker 3", "Them"])
        XCTAssertEqual(doc.people, ["Alex", "Speaker 2", "Speaker 3", "Them"])
        XCTAssertTrue(doc.isFinal)
        XCTAssertEqual(doc.durationMs, 30_003, "the audio's end")
        XCTAssertEqual(doc.line(atPlayhead: 11_000)?.text, "Can you share the budget?")
        XCTAssertNil(doc.line(atPlayhead: -1))
    }

    /// The far end's speakers the diarizer told apart, by label, in the order they first speak:
    /// each numbered by that order whether it has a name or not, so naming one never renumbers the
    /// others, and clearing a name brings back the same "Speaker N". The mic (You) and far-end lines
    /// without a label ("Them") carry no label: they cannot be named.
    func testEachDiarizedSpeakerKeepsItsNumberAndCarriesItsLabel() throws {
        let doc = try XCTUnwrap(document())
        XCTAssertEqual(doc.speakers.map(\.label), ["spk0", "spk1", "spk2"])
        XCTAssertEqual(doc.speakers.map(\.name), ["Alex", nil, nil])
        XCTAssertEqual(doc.speakers.map(\.number), [1, 2, 3])
        XCTAssertEqual(doc.speakers.map(\.display), ["Alex", "Speaker 2", "Speaker 3"])
        XCTAssertEqual(doc.ledger.map(\.label), ["spk0", nil, nil, "spk1", "spk2", nil])
        XCTAssertEqual(doc.speaker(labelled: "spk1")?.display, "Speaker 2")
        XCTAssertNil(doc.speaker(labelled: "spk9"))

        // Alex's name cleared: Speaker 1 again, and the others keep theirs.
        let unnamed = recordAnswer.replacingOccurrences(of: #"[{"speaker":"spk0","name":"Alex"}]"#, with: "[]")
        guard case .libraryRecord(let answer) = event(unnamed) else { return XCTFail("not a record") }
        let cleared = RecordDocument(answer)
        XCTAssertEqual(cleared.ledger.map(\.speaker.label), ["Speaker 1", "You", "You", "Speaker 2", "Speaker 3", "Them"])
        XCTAssertEqual(cleared.people, ["Speaker 1", "Speaker 2", "Speaker 3", "Them"])
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
            "said Speaker 2 @10000", "said Speaker 3 @18278",
        ])
    }

    func testOwedListsWhatStandsWithTheLineItWasSaidIn() throws {
        let doc = try XCTUnwrap(document())
        XCTAssertEqual(doc.owed.map(\.id), ["c2", "c3"], "the merged duplicate is folded away")
        XCTAssertEqual(doc.owed[0].citedLine?.text, "I'll send the revised plan by Friday.")
        XCTAssertEqual(doc.owed[1].citedLine?.speaker, .them("Speaker 2"))
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
        library.searchDelay = .zero
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
                  .libraryStats(sinceUnixMs: 0, ref: "c"), .speakerName(record: "r", speaker: "spk1", name: "A", ref: "d"),
                  .recordDelete(record: "r", ref: "e")] {
            XCTAssertNotNil(command(c.json)["id"] as? String, c.name)
        }
    }

    /// Review fix: a search waits for typing to pause (one question, not one per key), and asks
    /// with at most 200 characters.
    func testSearchWaitsForTypingToPauseAndIsCapped() async throws {
        let (library, sent) = model()
        library.searchDelay = .milliseconds(60)
        let searches = { sent().map(command).filter { $0["cmd"] as? String == "records.search" } }
        library.query = "b"
        library.query = "bu"
        library.query = "budget"
        XCTAssertEqual(searches().count, 0, "nothing while typing")
        XCTAssertEqual(library.searchLoad, .loading)
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(searches().count, 1)
        XCTAssertEqual(searches().last?["query"] as? String, "budget")

        library.query = String(repeating: "budget ", count: 60)
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(searches().count, 2)
        let sentQuery = searches().last?["query"] as? String ?? ""
        XCTAssertEqual(sentQuery.count, LibraryModel.maxQueryLength)
        XCTAssertTrue(sentQuery.hasPrefix("budget budget"))

        library.query = "zebra"
        library.query = ""
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(searches().count, 2, "cleared before the pause: nothing asked")
        XCTAssertEqual(library.searchLoad, .idle)
    }

    /// "Show older" asked, then the filter changed before the page came: the page belongs to the
    /// old list and is not appended to the new one (nor is its failure shown under it).
    func testAPageAskedForBeforeTheListReloadedIsNotAppended() {
        let (library, sent) = model()
        library.refreshList()
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(requestID(sent().last!))","more":true,"kind":"meeting","records":[\#(row("m1", start: 1_000, end: 2_000))]}
        """#)])
        library.loadMore()
        let olderPage = requestID(sent().last!)
        XCTAssertEqual(library.moreLoad, .loading)
        library.filter = .dictation
        let dictations = requestID(sent().last!)
        XCTAssertEqual(command(sent().last!)["kind"] as? String, "dictation")
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(dictations)","more":false,"kind":"dictation","records":[\#(row("d1", "dictation", start: 5_000, end: 5_100))]}
        """#)])
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(olderPage)","more":false,"kind":"meeting","records":[\#(row("m0", start: 500, end: 900))]}
        """#)])
        XCTAssertEqual(library.records.map(\.record), ["d1"], "the old list's page stays out")
        XCTAssertFalse(library.hasMore)
        library.apply([event(#"{"type":"command.failed","command":"records.list","id":"\#(olderPage)","message":"x"}"#)])
        XCTAssertEqual(library.moreLoad, .idle, "nor does its failure show under the new list")
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

    /// Naming a far-end speaker sends what the user typed, trimmed (empty clears the name), for the
    /// open record and that speaker's label; nothing when it did not change. The answer reads the
    /// record again (and Today's last meeting, when it is that one), so the name shows in the
    /// transcript and the header; a failure is said, never dropped.
    func testNamingASpeakerSendsTheNameAndTheRecordIsReadAgain() throws {
        let (library, sent) = model()
        library.nameSpeaker("spk1", "Robin")
        XCTAssertTrue(sent().isEmpty, "no record open: nothing to name")
        library.open("r1")
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])

        let before = sent().count
        library.nameSpeaker("spk1", "  Robin Example \n")
        let named = command(sent().last!)
        XCTAssertEqual(sent().count, before + 1)
        XCTAssertEqual(named["cmd"] as? String, "speaker.name")
        XCTAssertEqual(named["record"] as? String, "r1")
        XCTAssertEqual(named["speaker"] as? String, "spk1")
        XCTAssertEqual(named["name"] as? String, "Robin Example")
        let id = try XCTUnwrap(named["id"] as? String)

        // Unchanged: the same name, an unnamed speaker left empty, a label the record lacks, You.
        library.nameSpeaker("spk0", " Alex ")
        library.nameSpeaker("spk2", "   ")
        library.nameSpeaker("spk9", "Sam")
        library.nameSpeaker("", "Sam")
        XCTAssertEqual(sent().count, before + 1, "\(sent()[before...])")

        // On one line: a pasted line break or tab is a space.
        library.nameSpeaker("spk2", "Sam\n\tExample\u{0}")
        XCTAssertEqual(command(sent().last!)["name"] as? String, "Sam Example")
        XCTAssertEqual(LibraryModel.oneLine(" 👨‍👩‍👧 Ana  María "), "👨‍👩‍👧 Ana María", "emoji and accents stay")
        // Counted as the core counts: Unicode scalars, not what reads as one character.
        XCTAssertEqual(LibraryModel.nameLength(" 👨‍👩‍👧 Ana "), 9)
        XCTAssertEqual(LibraryModel.nameLength("e\u{301}"), 2)

        // Cleared: an empty name.
        library.nameSpeaker("spk0", "")
        let cleared = command(sent().last!)
        XCTAssertEqual(cleared["speaker"] as? String, "spk0")
        XCTAssertEqual(cleared["name"] as? String, "")

        let reads = sent().count
        library.apply([event(#"{"type":"speaker.named","record":"r1","speaker":"spk1","named":true,"ref":"\#(id)"}"#)])
        XCTAssertEqual(command(sent().last!)["cmd"] as? String, "record.open")
        XCTAssertEqual(sent().count, reads + 1)

        // Another record's speaker: the open one stays as it is.
        library.apply([event(#"{"type":"speaker.named","record":"r7","speaker":"spk1","named":true}"#)])
        XCTAssertEqual(sent().count, reads + 1)

        // A failure is said, and the next try clears it.
        library.nameSpeaker("spk2", "Zebra")
        let failedID = requestID(sent().last!)
        guard case .commandFailed(let failure) = event(
            #"{"type":"command.failed","command":"speaker.name","id":"\#(failedID)","message":"the library: disk I/O error"}"#)
        else { return XCTFail("not a failure") }
        XCTAssertTrue(library.handles(failure))
        library.apply([.commandFailed(failure)])
        XCTAssertEqual(library.namingFailure, "the library: disk I/O error")
        library.nameSpeaker("spk2", "Zebra Quartz")
        XCTAssertNil(library.namingFailure)
    }

    /// A rename sent before the record was read again is what an unchanged name is compared with:
    /// renaming straight back is sent, not taken for "unchanged" against the stale record. A
    /// failure forgets what was sent, and another record starts clean.
    func testNamingComparesWithWhatWasLastSentAndAFailureIsForgottenWithTheRecord() throws {
        let (library, sent) = model()
        library.open("r1")
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        let names = { sent().map(command).filter { $0["cmd"] as? String == "speaker.name" }.map { $0["name"] as? String } }

        library.nameSpeaker("spk0", "Alexandra")
        library.nameSpeaker("spk0", "Alex")
        XCTAssertEqual(names(), ["Alexandra", "Alex"], "back to the record's name, which is stale by now")
        library.nameSpeaker("spk0", "Alex")
        XCTAssertEqual(names().count, 2, "unchanged from what was last sent")

        library.nameSpeaker("spk1", "Robin")
        library.apply([event(#"{"type":"command.failed","command":"speaker.name","id":"\#(requestID(sent().last!))","message":"not found"}"#)])
        XCTAssertEqual(library.namingFailure, "not found")
        library.nameSpeaker("spk1", "Robin")
        XCTAssertEqual(names().last, "Robin", "a failed name is tried again")
        XCTAssertNil(library.namingFailure)

        library.apply([event(#"{"type":"command.failed","command":"speaker.name","id":"\#(requestID(sent().last!))","message":"not found"}"#)])
        library.open("r2")
        XCTAssertNil(library.namingFailure, "another record's failure is not this one's")
    }

    /// Today shows the last meeting whole: a speaker named in it is read again there too.
    func testNamingASpeakerOfTodaysLastMeetingReadsItAgain() throws {
        let (library, sent) = model()
        library.refreshToday()
        let listID = command(sent()[0])["id"] as! String
        library.apply([event(#"{"type":"library.records","ref":"\#(listID)","more":false,"kind":"meeting","records":[\#(row("r1", start: 5_000, end: 6_000))]}"#)])
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        XCTAssertEqual(library.lastMeeting?.record.record, "r1")
        let before = sent().count
        library.apply([event(#"{"type":"speaker.named","record":"r1","speaker":"spk1","named":true}"#)])
        let reread = command(sent().last!)
        XCTAssertEqual(sent().count, before + 1)
        XCTAssertEqual(reread["cmd"] as? String, "record.open")
        XCTAssertEqual(reread["record"] as? String, "r1")
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: reread["id"] as! String))])
        XCTAssertEqual(library.lastMeeting?.record.record, "r1", "read into Today's slot")
    }

    /// Deleting a record: asked for by id, with an id of its own. When the core says it is gone, it
    /// leaves the list and the matches; the selection moves to the record below it (the one above
    /// when it was the last), Today is read again (its last meeting may be the one that went), and
    /// what deleting left behind is said. A failure is said and nothing moves.
    func testADeletedRecordLeavesTheListAndTheSelectionMovesOn() throws {
        let (library, sent) = model()
        library.refreshList()
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(requestID(sent().last!))","more":false,"records":[
          \#(row("r3", start: 3_000, end: 4_000)), \#(row("r1", start: 2_000, end: 3_000)), \#(row("r0", start: 1_000, end: 2_000))]}
        """#)])
        library.open("r1")
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        XCTAssertNotNil(library.document)

        library.deleteRecord("r1")
        let asked = command(sent().last!)
        XCTAssertEqual(asked["cmd"] as? String, "record.delete")
        XCTAssertEqual(asked["record"] as? String, "r1")
        let id = try XCTUnwrap(asked["id"] as? String)
        XCTAssertEqual(library.records.map(\.record), ["r3", "r1", "r0"], "nothing moves before the core says so")

        // Refused (still live): said, and the record stays.
        guard case .commandFailed(let refused) = event(
            #"{"type":"command.failed","command":"record.delete","id":"\#(id)","message":"record r1 is still being finished"}"#)
        else { return XCTFail("not a failure") }
        XCTAssertTrue(library.handles(refused))
        library.apply([.commandFailed(refused)])
        XCTAssertEqual(library.deleteFailure, "record r1 is still being finished")
        XCTAssertEqual(library.selected, "r1")

        library.deleteRecord("r1")
        XCTAssertNil(library.deleteFailure, "a new try clears it")
        let again = requestID(sent().last!)
        let before = sent().count
        library.apply([event(#"{"type":"record.deleted","record":"r1","kind":"meeting","audio_left":false,"scrubbed":true,"ref":"\#(again)"}"#)])
        XCTAssertEqual(library.records.map(\.record), ["r3", "r0"])
        XCTAssertEqual(library.selected, "r0", "the record below it")
        XCTAssertNil(library.document, "until r0 is read")
        XCTAssertNil(library.deletionNote, "nothing left behind")
        let followed = sent()[before...].map(command)
        XCTAssertEqual(followed.filter { $0["cmd"] as? String == "record.open" }.map { $0["record"] as? String }, ["r0"],
                       "one read, of the record below")
        XCTAssertTrue(followed.contains { $0["cmd"] as? String == "library.stats" }, "Today's counts")
        XCTAssertTrue(followed.contains { $0["cmd"] as? String == "records.list" && $0["kind"] as? String == "meeting" },
                      "Today's last meeting")

        // The last one: the selection moves up. Left-behind words or audio are said.
        library.apply([event(#"{"type":"record.deleted","record":"r0","kind":"meeting","audio_left":true,"scrubbed":false}"#)])
        XCTAssertEqual(library.selected, "r3")
        let note = try XCTUnwrap(library.deletionNote)
        XCTAssertTrue(note.contains("words") && note.contains("recording"), note)
        // The only one: nothing is selected.
        library.apply([event(#"{"type":"record.deleted","record":"r3","kind":"meeting","audio_left":false,"scrubbed":true}"#)])
        XCTAssertTrue(library.records.isEmpty)
        XCTAssertNil(library.selected)
        XCTAssertNil(library.document)
        XCTAssertNil(library.deletionNote, "this one left nothing")
    }

    /// Deleted from the search: the selection moves among the matches the column shows, never to
    /// a record of the unsearched list. A failure that arrives once another record is open is not
    /// that record's to show. What a deletion left behind is said until the next record is opened,
    /// and Today waits for its next last meeting rather than reading as having none.
    func testDeletingFromTheSearchMovesAmongTheMatches() throws {
        let (library, sent) = model()
        library.refreshList()
        library.apply([event(#"""
        {"type":"library.records","ref":"\#(requestID(sent().last!))","more":false,"records":[
          \#(row("r9", start: 9_000, end: 9_500)), \#(row("r1", start: 5_000, end: 6_000))]}
        """#)])
        library.query = "budget"
        library.apply([event(#"""
        {"type":"library.search","ref":"\#(requestID(sent().last!))","query":"budget","hits":[
          {"record":"r2","title":"A","started_at_unix_ms":5000,"start_ms":0,"snippet":"budget"},
          {"record":"r2","title":"A","started_at_unix_ms":5000,"start_ms":9000,"snippet":"budget again"},
          {"record":"r4","title":"B","started_at_unix_ms":4000,"start_ms":0,"snippet":"the budget"}]}
        """#)])
        library.open("r2")
        library.apply([event(#"{"type":"record.deleted","record":"r2","kind":"meeting","audio_left":true,"scrubbed":true}"#)])
        XCTAssertEqual(library.hits.map(\.record), ["r4"])
        XCTAssertEqual(library.selected, "r4", "the next match, not r9 from the list")
        XCTAssertNotNil(library.deletionNote)

        library.deleteRecord("r4")
        let id = requestID(sent().last!)
        library.open("r1")
        XCTAssertNil(library.deletionNote, "said until the next record is opened")
        library.apply([event(#"{"type":"command.failed","command":"record.delete","id":"\#(id)","message":"record r4 is still being finished"}"#)])
        XCTAssertNil(library.deleteFailure, "r4's failure is not r1's to show")
    }

    func testTodayWaitsForItsNextLastMeeting() throws {
        let (library, sent) = model()
        library.refreshToday()
        let listID = command(sent()[0])["id"] as! String
        library.apply([event(#"{"type":"library.records","ref":"\#(listID)","more":false,"kind":"meeting","records":[\#(row("r1", start: 5_000, end: 6_000))]}"#)])
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        library.apply([event(#"{"type":"record.deleted","record":"r1","kind":"meeting","audio_left":false,"scrubbed":true}"#)])
        XCTAssertNil(library.lastMeeting)
        XCTAssertEqual(library.lastMeetingLoad, .loading, "not \"no meetings yet\" while it is asked for again")
    }

    /// A record deleted that is not the open one: the open one stays, and a match in it goes from
    /// the search; Today's last meeting, if it was that one, is gone.
    func testDeletingAnotherRecordKeepsTheOpenOneAndDropsItsMatches() throws {
        let (library, sent) = model()
        library.refreshToday()
        let listID = command(sent()[0])["id"] as! String
        library.apply([event(#"{"type":"library.records","ref":"\#(listID)","more":false,"kind":"meeting","records":[\#(row("r1", start: 5_000, end: 6_000))]}"#)])
        library.apply([event(recordAnswer.replacingOccurrences(of: "REQ", with: requestID(sent().last!)))])
        XCTAssertEqual(library.lastMeeting?.record.record, "r1")
        library.query = "budget"
        library.apply([event(#"""
        {"type":"library.search","ref":"\#(requestID(sent().last!))","query":"budget","hits":[
          {"record":"r1","title":"Plan","started_at_unix_ms":5000,"start_ms":10000,"snippet":"the budget"},
          {"record":"r2","title":"Other","started_at_unix_ms":4000,"start_ms":0,"snippet":"budget too"}]}
        """#)])
        library.open("r2")
        library.apply([event(#"{"type":"record.deleted","record":"r1","kind":"meeting","audio_left":false,"scrubbed":true}"#)])
        XCTAssertEqual(library.hits.map(\.record), ["r2"])
        XCTAssertEqual(library.selected, "r2")
        XCTAssertNil(library.lastMeeting, "Today's last meeting went; it is asked for again")
    }

    /// What the confirmation says goes, by kind: everything the record holds, and that it can't be
    /// undone; an imported file stays where the user keeps it.
    func testTheConfirmationSaysWhatGoes() throws {
        let rows = rows(#"{"type":"library.records","more":false,"records":[\#(row("m", start: 1)), \#(row("d", "dictation", start: 1)), \#(row("f", "file_import", start: 1))]}"#)
        let says = rows.map(LibraryModel.deletionWarning(for:))
        XCTAssertTrue(says[0].contains("audio") && says[0].contains("transcript") && says[0].contains("notes") && says[0].contains("owed"), says[0])
        XCTAssertTrue(says.allSatisfy { $0.contains("can't be undone") }, "\(says)")
        XCTAssertTrue(says[2].contains("file you imported stays"), says[2])
        XCTAssertFalse(says[1].contains("audio"), "a dictation keeps no audio: \(says[1])")
    }

    /// Inkwell 0.2's dictations came over: the list and the stats are read again.
    func testAnImportRefreshesWhatIsShown() {
        let (library, sent) = model()
        let before = sent().count
        library.apply([event(#"{"type":"import.finished","counts":{"dictations":2,"dictionary_entries":0,"snippets":0,"modes":0,"settings":0,"voice_commands":0,"app_style_rules":0,"linked_keys":0},"ref":"import.run"}"#)])
        let asked = sent()[before...].map { command($0)["cmd"] as? String }
        XCTAssertTrue(asked.contains("records.list") && asked.contains("library.stats"), "\(asked)")
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
        XCTAssertTrue(library.handles(stats), "Today says the counts could not be read")
        XCTAssertFalse(library.handles(other))
    }
}

// MARK: - Layout

@MainActor
final class LibraryLayoutTests: XCTestCase {
    /// The kind filters in the list column (272 pt, less its padding): every chip's label on one
    /// line, so the row is one chip high. A label squeezed into two lines ("Meetin/gs") is taller.
    func testTheKindFiltersStayOnOneLineInTheListColumn() {
        let hosting = NSHostingController(rootView: KindFilter().environment(LibraryModel(send: { _ in })))
        let fitted = hosting.sizeThatFits(in: CGSize(width: 272 - 2 * 12 - 2 * 6, height: 400))
        XCTAssertLessThanOrEqual(fitted.height, 28.5, "one line of chips, 28 pt high")
        XCTAssertLessThanOrEqual(fitted.width, 236.5, "within the column")
        // Narrower than the chips: still one line, scrolling sideways.
        let narrow = hosting.sizeThatFits(in: CGSize(width: 150, height: 400))
        XCTAssertLessThanOrEqual(narrow.height, 28.5)
        XCTAssertLessThanOrEqual(narrow.width, 150.5)
    }
}
