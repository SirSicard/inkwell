// The wandering orb, drawing through the real pipeline and display link: live it drifts on the
// frames it already draws; at rest it glides to a new spot when it comes on screen, when the
// screen behind it changes and now and then, and between those it draws nothing. The view is
// treated as on screen without a window (assumeOnScreen), so this runs with the screen locked too.
import AppKit
import XCTest

@testable import InkRenderer

@MainActor
final class WanderViewTests: XCTestCase {
    private let bounds = OrbWander.Bounds(x: 0.46...0.68, y: 0.18...0.36)

    private func makeView(reduceMotion: Bool) -> InkView {
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 120, height: 80))
        view.assumeReduceMotion = reduceMotion
        view.wanderRandom = .seeded(11)
        view.placement = OrbPlacement(x: 0.56, yFromTop: 0.26, unit: 0.72)
        view.wanderBounds = bounds
        return view
    }

    /// Puts the view on screen, once the pipeline is ready (until then it draws nothing).
    private func show(_ view: InkView) async throws {
        _ = try InkPipelineLoader.shared.wait().get()
        try await spin(0.05)
        XCTAssertTrue(view.isReady)
        view.assumeOnScreen = true
        try await spin(0.3)
    }

    /// At rest, coming on screen glides the orb to a new spot for OrbWander.restGlide; then the
    /// display link stops, and nothing more is drawn.
    func testAtRestItGlidesWhenShownThenDrawsNothing() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = makeView(reduceMotion: false)
        let start = view.orbCentre
        try await show(view)
        XCTAssertTrue(view.isAnimating, "gliding to its new spot")
        try await spin(OrbWander.restGlide + 0.4)
        XCTAssertFalse(view.isAnimating, "arrived: the display link stops")
        XCTAssertNotEqual(view.orbCentre, start)
        XCTAssertTrue(bounds.contains(view.orbCentre))
        let settled = view.framesDrawn
        print("wander: \(settled) frames for one rest glide of \(OrbWander.restGlide) s")
        try await spin(0.6)
        XCTAssertEqual(view.framesDrawn, settled, "at rest: nothing drawn between moves")
    }

    /// Motion off (Reduce Motion, or "Always still"): the same moments take a new spot at once, in
    /// the one still frame each already draws; no display link.
    func testWithMotionOffItMovesWithoutAGlide() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = makeView(reduceMotion: true)
        let start = view.orbCentre
        try await show(view)
        XCTAssertFalse(view.isAnimating)
        XCTAssertEqual(view.framesDrawn, 1, "shown: one still frame, already at the new spot")
        let shown = view.orbCentre
        XCTAssertNotEqual(shown, start)

        view.contentID = "library"
        XCTAssertFalse(view.isAnimating)
        XCTAssertEqual(view.framesDrawn, 2, "another screen: one still frame at another spot")
        XCTAssertNotEqual(view.orbCentre, shown)
        try await spin(0.3)
        XCTAssertEqual(view.framesDrawn, 2)
    }

    /// The screen behind it changing, at rest: a glide to a new spot.
    func testAChangeOfScreenAtRestPicksANewSpot() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = makeView(reduceMotion: false)
        view.contentID = "today"
        try await show(view)
        try await spin(OrbWander.restGlide + 0.4)
        let before = view.orbCentre
        view.contentID = "today"
        XCTAssertFalse(view.isAnimating, "the same screen: it stays")
        view.contentID = "settings"
        XCTAssertTrue(view.isAnimating)
        try await spin(OrbWander.restGlide + 0.4)
        XCTAssertFalse(view.isAnimating)
        XCTAssertNotEqual(view.orbCentre, before)
    }

    /// The user coming back to the window (it becomes key) moves a resting orb that has held its
    /// spot for restInterval; sooner, or hidden, it stays. Nothing moves it on a timer: left alone
    /// it draws nothing.
    func testComingBackToTheWindowMovesItAfterAWhileOnlyWhileOnScreen() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = makeView(reduceMotion: true)
        view.restInterval = 0.4
        // In a window that is never ordered in: the view hears its window's notices, and counts as
        // on screen by assumeOnScreen.
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 120, height: 80), styleMask: [.borderless],
                              backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.contentView = view
        try await show(view)
        func becomeKey() { NotificationCenter.default.post(name: NSWindow.didBecomeKeyNotification, object: window) }
        let shown = view.orbCentre, framesShown = view.framesDrawn
        becomeKey()
        XCTAssertEqual(view.orbCentre, shown, "just moved: it stays")
        try await spin(0.6)
        XCTAssertEqual(view.framesDrawn, framesShown, "no timer: nothing drawn while left alone")
        becomeKey()
        XCTAssertNotEqual(view.orbCentre, shown, "back after a while: a new spot")
        XCTAssertEqual(view.framesDrawn, framesShown + 1)
        try await spin(0.6)
        view.assumeOnScreen = false
        let hidden = view.orbCentre, frames = view.framesDrawn
        becomeKey()
        XCTAssertEqual(view.orbCentre, hidden, "hidden: it never moves")
        XCTAssertEqual(view.framesDrawn, frames)
        window.contentView = nil
    }

    /// Live, it drifts on the frames it already draws; going back to rest leaves it where it got to.
    func testLiveItDriftsAndAtRestItStaysWhereItGot() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = makeView(reduceMotion: false)
        try await show(view)
        try await spin(OrbWander.restGlide + 0.4)
        let rest = view.orbCentre
        view.state = .dictating
        try await spin(1.0)
        let drifted = view.orbCentre
        XCTAssertNotEqual(drifted, rest, "live: it drifts")
        view.state = .idle
        XCTAssertFalse(view.isAnimating)
        XCTAssertEqual(view.orbCentre, drifted, accuracy: 0.002, "no jump back on going to rest")
    }

    /// Without bounds (the Drop, the first run) it stays at its placement, whatever happens.
    func testWithoutBoundsItStaysPut() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 120, height: 80))
        view.assumeReduceMotion = false
        view.placement = OrbPlacement(x: 0.5, yFromTop: 0.5, unit: 1)
        try await show(view)
        view.contentID = "elsewhere"
        XCTAssertFalse(view.isAnimating)
        XCTAssertEqual(view.orbCentre, SIMD2(0.5, 0.5))
        XCTAssertEqual(view.framesDrawn, 1)
    }

    private func spin(_ seconds: Double) async throws {
        try await Task.sleep(for: .seconds(seconds))
    }
}

private func XCTAssertEqual(
    _ a: SIMD2<Double>, _ b: SIMD2<Double>, accuracy: Double, _ message: String, file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertEqual(a.x, b.x, accuracy: accuracy, message, file: file, line: line)
    XCTAssertEqual(a.y, b.y, accuracy: accuracy, message, file: file, line: line)
}
