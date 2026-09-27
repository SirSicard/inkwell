// The hop from the core's event thread to the main actor: in order, in batches, never waiting on
// the main thread, and no timer (a hand-over is queued by an event and by nothing else).
import Foundation
import InkBridge
import XCTest

/// Decodes one event as the core sends it; fails the test if it does not decode to a known event.
func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    if case .undecodable = decoded {
        XCTFail("undecodable: \(json)", file: file, line: line)
    }
    return decoded
}

func finalEvent(_ n: Int, channel: String = "mic", record: String = "r1") -> InkEvent {
    event(#"{"type":"meeting.final","record":"\#(record)","channel":"\#(channel)","start_ms":\#(n),"end_ms":\#(n + 1),"text":"line \#(n)"}"#)
}

func partialEvent(_ text: String, channel: String = "mic", record: String = "r1") -> InkEvent {
    event(#"{"type":"meeting.partial","record":"\#(record)","channel":"\#(channel)","text":"\#(text)"}"#)
}

/// What a relay delivered, batch by batch.
@MainActor
final class Delivered {
    var batches: [[InkEvent]] = []
    var all: [InkEvent] { batches.flatMap { $0 } }
}

/// Runs every block already on the main queue (the relay's hand-overs among them): the main
/// queue is serial, so the block queued last runs after them.
@MainActor
func drainMainQueue(_ test: XCTestCase) {
    let drained = test.expectation(description: "main queue drained")
    DispatchQueue.main.async { drained.fulfill() }
    test.wait(for: [drained], timeout: 5)
}

final class EventRelayTests: XCTestCase {
    /// Pushes `events` from a background thread while the main thread waits, so no hand-over can
    /// run until they are all queued. A push that waited for the main thread would deadlock here.
    @MainActor
    private func pushWhileMainIsBusy(_ relay: EventRelay, _ events: [InkEvent]) {
        let pushed = DispatchGroup()
        pushed.enter()
        DispatchQueue.global().async {
            for event in events {
                relay.push(event)
            }
            pushed.leave()
        }
        XCTAssertEqual(pushed.wait(timeout: .now() + 10), .success, "push waited for the main thread")
    }

    @MainActor
    func testABurstFromTheEventThreadArrivesInOrderInOneHandOver() {
        let delivered = Delivered()
        let relay = EventRelay { delivered.batches.append($0) }
        let burst = (0..<1000).map { finalEvent($0) }

        pushWhileMainIsBusy(relay, burst)
        XCTAssertEqual(relay.waitingCount, 1000)
        XCTAssertTrue(delivered.batches.isEmpty, "nothing reaches the main actor before it runs")

        drainMainQueue(self)
        XCTAssertEqual(relay.handOvers, 1, "one main-thread wake-up for the whole burst")
        XCTAssertEqual(delivered.batches.count, 1)
        XCTAssertEqual(delivered.all, burst, "every event, in the order the core sent it")
        XCTAssertEqual(relay.waitingCount, 0)
    }

    @MainActor
    func testAnEventAfterAHandOverQueuesTheNextOne() {
        let delivered = Delivered()
        let relay = EventRelay { delivered.batches.append($0) }

        pushWhileMainIsBusy(relay, [finalEvent(1)])
        drainMainQueue(self)
        pushWhileMainIsBusy(relay, [finalEvent(2), finalEvent(3)])
        drainMainQueue(self)

        XCTAssertEqual(delivered.batches, [[finalEvent(1)], [finalEvent(2), finalEvent(3)]])
        XCTAssertEqual(relay.handOvers, 2)
    }

    @MainActor
    func testNothingIsQueuedForTheMainActorWhileTheCoreIsQuiet() {
        let delivered = Delivered()
        let relay = EventRelay { delivered.batches.append($0) }
        drainMainQueue(self)
        XCTAssertTrue(delivered.batches.isEmpty, "no event, no hand-over, no empty batch")
        XCTAssertEqual(relay.handOvers, 0)
        XCTAssertEqual(relay.waitingCount, 0)
    }

    @MainActor
    func testANewerPartialReplacesTheWaitingOneAndArrivesAfterEarlierFinals() {
        let delivered = Delivered()
        let relay = EventRelay { delivered.batches.append($0) }

        pushWhileMainIsBusy(relay, [
            partialEvent("a"),
            finalEvent(1),
            partialEvent("b"),
            partialEvent("x", channel: "far"),
            partialEvent("c"),
            partialEvent("other meeting", record: "r2"),
        ])
        drainMainQueue(self)

        XCTAssertEqual(delivered.all, [
            finalEvent(1),
            partialEvent("x", channel: "far"),
            partialEvent("c"),
            partialEvent("other meeting", record: "r2"),
        ], "only the latest partial per record and channel, behind the final queued before it")
    }

    @MainActor
    func testPushesFromManyThreadsAllArrive() {
        let delivered = Delivered()
        let relay = EventRelay { delivered.batches.append($0) }
        // Blocks the main thread until every push is done.
        DispatchQueue.concurrentPerform(iterations: 8) { thread in
            for n in 0..<250 {
                relay.push(finalEvent(thread * 1000 + n))
            }
        }
        drainMainQueue(self)
        let starts = delivered.all.compactMap { if case .meetingFinal(let f) = $0 { f.startMs } else { nil } }
        XCTAssertEqual(starts.count, 2000)
        XCTAssertEqual(Set(starts).count, 2000, "no event lost or delivered twice")
        // Each thread's events keep their own order.
        for thread in 0..<8 {
            let own = starts.filter { $0 / 1000 == Int64(thread) }
            XCTAssertEqual(own, own.sorted())
        }
    }
}
