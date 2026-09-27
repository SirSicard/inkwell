// The view in a real window: a display link only while live, one still frame on entering idle,
// and nothing after it. Skipped where no window can be on screen (a locked screen, no display).
import AppKit
import XCTest

@testable import InkRenderer

@MainActor
final class InkViewTests: XCTestCase {
    private var window: NSWindow?

    override func tearDown() async throws {
        await MainActor.run {
            window?.orderOut(nil)
            window = nil
        }
    }

    func testIdleDrawsOneStillFrameLiveRunsAtTheDisplayRateAndIdleStopsAgain() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 84, height: 84))
        let window = NSWindow(contentRect: NSRect(x: 80, y: 80, width: 84, height: 84), styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.level = .floating
        window.contentView = view
        window.orderFrontRegardless()
        self.window = window
        try await spin(0.3)
        try XCTSkipUnless(window.occlusionState.contains(.visible), "the window is not on screen (locked or headless)")

        XCTAssertEqual(view.framesDrawn, 1, "idle on screen: one still frame")
        XCTAssertFalse(view.isAnimating)
        try await spin(0.5)
        XCTAssertEqual(view.framesDrawn, 1, "and nothing after it")

        let before = InkRenderer.frames.count
        view.state = .meeting
        XCTAssertTrue(view.isAnimating)
        try await spin(1.0)
        let live = view.framesDrawn - 1
        XCTAssertGreaterThan(live, 30, "live: about 60 frames a second")
        XCTAssertLessThan(live, 75, "no faster than 60")
        XCTAssertEqual(InkRenderer.frames.count - before, UInt64(live), "every frame is counted")

        view.state = .idle
        XCTAssertFalse(view.isAnimating, "idle stops the display link")
        let settled = view.framesDrawn
        try await spin(0.5)
        XCTAssertEqual(view.framesDrawn, settled, "0 frames while idle")
    }

    func testACoveredViewDrawsNothing() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 84, height: 84))
        view.state = .dictating
        try await spin(0.3)
        XCTAssertEqual(view.framesDrawn, 0, "not in a window")
        XCTAssertFalse(view.isAnimating)
    }

    /// The Reduce Motion observer lives exactly as long as the view is in a window, like the
    /// occlusion observer: a view taken out of its window listens to nothing.
    func testTheReduceMotionObserverIsPairedWithTheWindow() {
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 84, height: 84))
        XCTAssertFalse(view.observesDisplayOptions, "not before it is in a window")
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 84, height: 84), styleMask: [.borderless],
                              backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.contentView = view
        XCTAssertTrue(view.observesDisplayOptions, "in a window")
        window.contentView = NSView()
        XCTAssertFalse(view.observesDisplayOptions, "removed with the window")
        window.contentView = view
        XCTAssertTrue(view.observesDisplayOptions, "and added again, once")
    }

    private func spin(_ seconds: Double) async throws {
        try await Task.sleep(for: .seconds(seconds))
    }
}
