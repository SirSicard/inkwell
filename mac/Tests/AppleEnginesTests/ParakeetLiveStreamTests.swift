// One live stream with a scripted decoder: pushes that never wait, words that arrive in order,
// a finish answered after its last final, a close that ends everything, stalls and failures
// reported. Run under Thread Sanitizer too (swift test --sanitize=thread).
@testable import AppleEngines
import Foundation
import InkBridge
import Synchronization
import XCTest

/// A decoder that fails every decode.
private struct FailingDecoder: WindowDecoder {
    func decode(_ samples: [Float]) async throws(ParakeetError) -> DecodedWindow {
        throw .decodeFailed(code: 7)
    }
}

/// A decoder whose decodes hold until the test releases them, and that says when one began.
private final class GatedDecoder: WindowDecoder {
    let began = Signal<Bool>()
    private let released = Atomic(false)
    private let finished = Counter()

    func decode(_ samples: [Float]) async throws(ParakeetError) -> DecodedWindow {
        began.set(true)
        while !released.load(ordering: .acquiring) {
            try? await Task.sleep(for: .milliseconds(5))
        }
        finished.add()
        return DecodedWindow(words: [])
    }

    func release() { released.store(true, ordering: .releasing) }

    /// Decodes that have returned.
    var decodesFinished: Int { finished.value }
}

/// Pushes `range` of indexed audio in 20 ms blocks from a thread of its own, as the core's worker.
private func push(_ stream: ParakeetLiveStream, _ range: Range<Int>) throws(InkEngineError) {
    var t = range.lowerBound
    while t < range.upperBound {
        let n = min(320, range.upperBound - t)
        try stream.push(indexed(t..<t + n))
        t += n
    }
}

final class ParakeetLiveStreamTests: XCTestCase {
    private func stream(_ decoder: any WindowDecoder, _ sink: RecordingSink) -> ParakeetLiveStream {
        ParakeetLiveEngine(decoder: decoder).stream(channel: .mic, sink: sink)
    }

    func testWordsArriveAsPartialsAndFinishAnswersAfterTheLastFinal() throws {
        let sink = RecordingSink()
        let script = unbroken(8)  // 0.5 s to about 2.9 s, then nothing until the end
        let live = stream(ScriptedDecoder(script: script), sink)
        let worker = DispatchQueue(label: "core-worker")
        let pushed = Signal<Bool>()
        worker.async {
            pushed.set((try? push(live, 0..<48_000)) != nil)
        }
        XCTAssertEqual(pushed.wait(), true)
        let answered = Signal<(Result<Void, InkEngineError>, Int)>()
        live.finish { result in answered.set((result, sink.finals.count)) }
        let (result, finalsWhenAnswered) = try XCTUnwrap(answered.wait())
        XCTAssertNoThrow(try result.get())
        XCTAssertEqual(finalsWhenAnswered, 1, "the final was sent before finish answered")
        XCTAssertEqual(sink.finals.map(\.text), [script.map(\.text).joined(separator: " ")])
        XCTAssertFalse(sink.partials.filter { !$0.isEmpty }.isEmpty, "partials came first")
        live.close()
    }

    func testClosingAnswersAWaitingFinishAndNothingIsSentAfterIt() throws {
        let sink = RecordingSink()
        let live = stream(ScriptedDecoder(script: unbroken(4), delay: .milliseconds(300)), sink)
        try push(live, 0..<24_000)
        let answered = Signal<Result<Void, InkEngineError>>()
        live.finish { answered.set($0) }
        live.close()
        let result = try XCTUnwrap(answered.wait())
        XCTAssertEqual(result.failureValue, .cancelled)
        let count = sink.all.count
        Thread.sleep(forTimeInterval: 0.8)
        XCTAssertEqual(sink.all.count, count, "a decode running at close sends nothing")
        XCTAssertThrowsError(try push(live, 24_000..<24_320), "no audio after close")
    }

    func testADecodeThatFallsBehindIsReportedAsAStallOnce() throws {
        let sink = RecordingSink()
        let decoder = ScriptedDecoder(script: [], delay: .milliseconds(800))
        let live = stream(decoder, sink)
        try push(live, 0..<(5 * 16_000))  // 5 s of audio at once, while the first decode runs
        XCTAssertEqual(sink.all.filter { $0 == .stalled(1) }.count, 1)
        live.close()
    }

    /// Every push returns while a decode is held: an order, not a time, so a slow runner (Thread
    /// Sanitizer on CI) cannot fail it.
    func testPushesNeverWaitForADecode() throws {
        let sink = RecordingSink()
        let decoder = GatedDecoder()
        let live = stream(decoder, sink)
        defer {
            decoder.release()
            live.close()
        }
        // Armed before the first push: a push that waited for the decode (the one it starts, or
        // one already running) would return only after this releases it, and then after the
        // decode, so the check below fails on that order instead of the test hanging.
        DispatchQueue.global().asyncAfter(deadline: .now() + 30) { decoder.release() }
        // Half a second starts the first decode, which then holds until it is released.
        try push(live, 0..<8_000)
        XCTAssertEqual(decoder.began.wait(30), true, "a decode began")
        for i in 25..<225 {
            try live.push(indexed(i * 320..<(i + 1) * 320))
        }
        XCTAssertEqual(decoder.decodesFinished, 0, "every push returned while the decode was held")
    }

    func testAFailedDecodeEndsTheStream() throws {
        let sink = RecordingSink()
        let live = stream(FailingDecoder(), sink)
        // The first decode starts after half a second; once it has failed, the next push is
        // refused with its code.
        let deadline = Date().addingTimeInterval(5)
        var refused: InkEngineError?
        var t = 0
        while refused == nil, Date() < deadline {
            do throws(InkEngineError) {
                try live.push(indexed(t..<t + 320))
                t += 320
            } catch {
                refused = error
            }
            if t > 16_000 { Thread.sleep(forTimeInterval: 0.01) }
        }
        XCTAssertEqual(refused, .failed(code: 207))
        let answered = Signal<Result<Void, InkEngineError>>()
        live.finish { answered.set($0) }
        XCTAssertEqual(try XCTUnwrap(answered.wait()).failureValue, .failed(code: 207))
        XCTAssertTrue(sink.finals.isEmpty, "nothing made up")
        live.close()
    }
}

extension Result {
    var failureValue: Failure? {
        if case .failure(let e) = self { e } else { nil }
    }
}
