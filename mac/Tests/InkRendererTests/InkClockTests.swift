// One display link drives every live ink: with the Drop and the rail both live, one callback per
// vsync draws both, instead of two links each waking the main thread. Each view's schedule still
// decides whether it takes part.
import AppKit
import XCTest

@testable import InkRenderer

@MainActor
final class InkClockTests: XCTestCase {
    private func views(on clock: InkClock) throws -> (InkView, InkView) {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let loader = InkPipelineLoader { try InkPipeline() }
        _ = loader.wait()
        let drop = InkView(frame: NSRect(x: 0, y: 0, width: 96, height: 84), loader: loader, clock: clock)
        let rail = InkView(frame: NSRect(x: 0, y: 0, width: 56, height: 700), loader: loader, clock: clock)
        drop.assumeOnScreen = true
        rail.assumeOnScreen = true
        return (drop, rail)
    }

    func testTwoLiveViewsShareOneLinkAndBothDrawOnEachTick() throws {
        let clock = InkClock()
        let (drop, rail) = try views(on: clock)
        XCTAssertEqual(drop.framesDrawn, 1, "idle on screen: one still frame")
        XCTAssertEqual(rail.framesDrawn, 1)
        XCTAssertEqual(clock.linkCount, 0, "nothing live, no link")

        drop.state = .meeting
        rail.state = .meeting
        XCTAssertEqual(clock.clientCount, 2)
        XCTAssertEqual(clock.linkCount, NSScreen.screens.isEmpty ? 0 : 1, "one link for both (none without a screen)")
        XCTAssertTrue(drop.isAnimating)
        XCTAssertTrue(rail.isAnimating)

        let before = InkRenderer.frames.count
        for tick in 1...3 {
            clock.tick(at: Double(tick) / 60)
        }
        XCTAssertEqual(drop.framesDrawn, 1 + 3, "the Drop drew on every tick")
        XCTAssertEqual(rail.framesDrawn, 1 + 3, "and so did the rail")
        XCTAssertEqual(InkRenderer.frames.count - before, 6)
    }

    func testWhenOnlyOneViewIsLiveOnlyItDraws() throws {
        let clock = InkClock()
        let (drop, rail) = try views(on: clock)
        drop.state = .dictating
        XCTAssertEqual(clock.clientCount, 1)
        clock.tick(at: 1.0 / 60)
        clock.tick(at: 2.0 / 60)
        XCTAssertEqual(drop.framesDrawn, 1 + 2)
        XCTAssertEqual(rail.framesDrawn, 1, "the idle rail keeps its still frame")

        rail.state = .meeting
        drop.state = .idle
        XCTAssertEqual(drop.framesDrawn, 1 + 2 + 1, "leaving the clock: one settled frame")
        XCTAssertEqual(clock.clientCount, 1)
        clock.tick(at: 3.0 / 60)
        XCTAssertEqual(drop.framesDrawn, 4, "an idle view draws nothing on a tick")
        XCTAssertEqual(rail.framesDrawn, 2)
    }

    func testTheLinkGoesWithTheLastLiveView() throws {
        let clock = InkClock()
        let (drop, rail) = try views(on: clock)
        drop.state = .meeting
        rail.state = .meeting
        drop.state = .idle
        XCTAssertEqual(clock.clientCount, 1)
        XCTAssertEqual(clock.linkCount, NSScreen.screens.isEmpty ? 0 : 1, "still running for the rail")
        rail.state = .idle
        XCTAssertEqual(clock.clientCount, 0)
        XCTAssertEqual(clock.linkCount, 0, "no live view, no link")
    }

    func testAViewThatGoesOffScreenLeavesTheClock() throws {
        let clock = InkClock()
        let (drop, rail) = try views(on: clock)
        drop.state = .meeting
        rail.state = .meeting
        rail.assumeOnScreen = false
        XCTAssertEqual(clock.clientCount, 1)
        let drawn = rail.framesDrawn
        clock.tick(at: 1.0 / 60)
        XCTAssertEqual(rail.framesDrawn, drawn, "a covered view draws nothing")
        drop.state = .idle
        XCTAssertEqual(clock.linkCount, 0)
    }
}
