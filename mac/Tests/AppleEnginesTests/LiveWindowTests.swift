// The trailing-window scheme's decisions, driven with scripted speech. Each lesson from the
// earlier FluidAudio engine (LiveWindow.swift's header) has its own test here.
@testable import AppleEngines
import InkBridge
import XCTest

/// `window` decoded as the words of `script` fully inside it (the scheme sees stream indices).
private func decode(_ window: Window, _ script: [ScriptWord]) -> DecodedWindow {
    DecodedWindow(words: script.filter { $0.start >= window.start && $0.end <= window.end }.map {
        TimedWord(text: $0.text, start: $0.start - window.start, end: $0.end - window.start)
    })
}

/// Feeds `seconds` of stream in blocks of `block`, decoding whenever the scheme wants to, then
/// ends the stream. Each output comes with the stream sample at which it was sent.
private func drive(
    _ script: [ScriptWord], seconds: Double, block: Int = 1_600,
    config: LiveWindowConfig = LiveWindowConfig(),
    inspect: (LiveWindow) -> Void = { _ in }
) -> [(at: Int, output: LiveOutput)] {
    var live = LiveWindow(config: config)
    var sent: [(at: Int, output: LiveOutput)] = []
    let total = Int(seconds * 16_000)
    var t = 0
    while t < total {
        let n = min(block, total - t)
        live.append(indexed(t..<t + n))
        t += n
        if live.wantsDecode {
            let window = live.takeWindow()
            sent += live.apply(decode(window, script), of: window).map { (t, $0) }
            inspect(live)
        }
    }
    if let window = live.takeLastWindow() {
        sent += live.applyLast(decode(window, script), of: window).map { (t, $0) }
    } else {
        sent += live.endWithoutWindow().map { (t, $0) }
    }
    return sent
}

private func finals(_ sent: [(at: Int, output: LiveOutput)]) -> [(at: Int, segment: InkSegment)] {
    sent.compactMap { if case .final(let s) = $0.output { ($0.at, s) } else { nil } }
}

private func words(_ text: String) -> [String] {
    text.split(separator: " ").map(String.init)
}

final class LiveWindowTests: XCTestCase {
    func testAPauseAfterTheLastWordSettlesTheUtterance() {
        let script = unbroken(5)  // 0.5 s to about 1.95 s
        let sent = drive(script, seconds: 4)
        let settled = finals(sent)
        XCTAssertEqual(settled.count, 1)
        XCTAssertEqual(settled.first?.segment.text, "w0 w1 w2 w3 w4")
        XCTAssertEqual(settled.first?.segment.startMs, 500)
        XCTAssertEqual(settled.first?.segment.endMs, UInt64(script[4].end / 16))
        // Settled once the pause is heard (0.8 s after the last word), not at the stream's end.
        XCTAssertLessThan(settled[0].at, script[4].end + 12_800 + 8_000 + 1_600)
        XCTAssertEqual(sent.last?.output, .partial(""), "the partial is cleared")
    }

    /// The earlier engine's end-of-utterance model fired only on a pause: 25 s of unbroken speech
    /// produced no final at all. Here unbroken speech is settled by length.
    func testUnbrokenSpeechIsSettledWithoutWaitingForAPause() {
        let script = unbroken(100)  // 30 s without a pause
        let settled = finals(drive(script, seconds: 31))
        let beforeTheEnd = settled.filter { $0.at < script.last!.end }
        XCTAssertGreaterThanOrEqual(beforeTheEnd.count, 2, "settled while still speaking")
        // The first settles once the utterance reaches 12 s, give or take a hop.
        XCTAssertLessThanOrEqual(settled[0].at, 8_000 + 192_000 + 8_000)
        for final in settled {
            XCTAssertLessThanOrEqual(final.segment.endMs - final.segment.startMs, 12_000)
        }
        XCTAssertEqual(settled.flatMap { words($0.segment.text) }, script.map(\.text), "every word, once")
    }

    /// The earlier engine re-armed its force-commit timer on every push, and pushes never stop
    /// (silence is audio too), so it never fired. Here the bound is audio since the utterance
    /// began: how the audio is pushed cannot move it.
    func testHowAudioIsPushedCannotPostponeTheBound() {
        let script = unbroken(100)
        var first: [Int] = []
        for block in [160, 5_120, 16_000] {
            let settled = finals(drive(script, seconds: 31, block: block))
            XCTAssertFalse(settled.isEmpty)
            first.append(settled[0].at)
            XCTAssertLessThanOrEqual(settled[0].at, 8_000 + 192_000 + 8_000 + block, "block \(block)")
        }
        XCTAssertLessThanOrEqual(first.max()! - first.min()!, 16_000, "\(first)")
    }

    /// The earlier engine's callbacks carried the transcript accumulated since the session began,
    /// so each utterance repeated all before it. Here a final carries only its own words, and a
    /// partial only words not settled yet.
    func testFinalsCarryOnlyTheirOwnWordsAndPartialsOnlyUnsettledOnes() {
        let script = unbroken(6, from: 8_000, prefix: "a") + unbroken(4, from: 80_000, prefix: "b")
            + unbroken(5, from: 144_000, prefix: "c")
        let sent = drive(script, seconds: 14)
        XCTAssertEqual(finals(sent).map(\.segment.text), [
            "a0 a1 a2 a3 a4 a5", "b0 b1 b2 b3", "c0 c1 c2 c3 c4",
        ])
        let utterances = ["a", "b", "c", "none after the last"]
        var settledSoFar = 0
        for (_, output) in sent {
            switch output {
            case .final:
                // After a final, only the next utterance's words may show.
                settledSoFar += 1
            case .partial(let p):
                let utterance = utterances[settledSoFar]
                XCTAssertTrue(words(p).allSatisfy { $0.hasPrefix(utterance) }, "\(p) during \(utterance)")
            }
        }
    }

    /// FluidAudio's ASRResult.duration is 0 past one 15 s model window. Final times here come from
    /// sample counts: an utterance 20 s in is placed at 20 s, whatever the decoder reports.
    func testFinalTimesAreCountedInSamples() {
        let script = unbroken(3, from: 320_000)  // after 20 s of silence
        let settled = finals(drive(script, seconds: 24))
        XCTAssertEqual(settled.count, 1)
        XCTAssertEqual(settled[0].segment.startMs, 20_000)
        XCTAssertEqual(settled[0].segment.endMs, UInt64(script[2].end / 16))
    }

    func testSilenceKeepsTheWindowShort() {
        var longest = 0
        _ = drive([], seconds: 30) { longest = max(longest, $0.buffer.count) }
        let config = LiveWindowConfig()
        XCTAssertLessThanOrEqual(longest, config.keepSilence + config.hop)
    }

    /// A decoder far behind must not make memory hold the session (architecture rule 3).
    func testABufferPastItsCapLetsTheOldestAudioGo() {
        var live = LiveWindow()
        let window = live.takeWindow()
        for i in 0..<120 {  // two minutes, none of it decoded
            live.append(indexed(i * 16_000..<(i + 1) * 16_000))
        }
        XCTAssertEqual(live.buffer.count, live.config.maxBuffer)
        XCTAssertEqual(live.droppedUnheard, 120 * 16_000 - live.config.maxBuffer)
        XCTAssertEqual(live.apply(DecodedWindow(words: []), of: window), [], "a stale decode is ignored")
        XCTAssertEqual(live.buffer.count, live.config.maxBuffer)
    }

    func testTheStreamsEndSettlesWhatIsLeft() {
        let script = unbroken(10)
        // The stream ends 0.1 s after the last word: no pause heard.
        let sent = drive(script, seconds: Double(script.last!.end + 1_600) / 16_000)
        let settled = finals(sent)
        XCTAssertEqual(settled.flatMap { words($0.segment.text) }, script.map(\.text))
        XCTAssertEqual(sent.last?.output, .partial(""))
    }

    func testPartialsChangeOnlyWhenTheWordsDo() {
        let sent = drive(unbroken(20), seconds: 8)
        let partials = sent.compactMap { if case .partial(let p) = $0.output { p } else { nil } }
        XCTAssertGreaterThan(partials.count, 5)
        for (a, b) in zip(partials, partials.dropFirst()) {
            XCTAssertNotEqual(a, b, "a partial is sent only when it changes")
        }
    }
}
