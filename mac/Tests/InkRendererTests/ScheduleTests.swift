// When the ink draws: architecture rule 9 ("draw nothing when idle") as the view's decisions.
// The view runs a display link only while a live state is on screen with motion allowed; every
// other change draws at most one still frame, and only when the one on screen is out of date.
import XCTest

@testable import InkRenderer

final class ScheduleTests: XCTestCase {
    func testIdleOnScreenDrawsOneStillFrameAndNeverRunsTheClock() {
        var s = InkSchedule()
        XCTAssertEqual(s.set(onScreen: true), .drawStill)
        XCTAssertEqual(s.update(), .nothing, "the still frame is current: nothing more to draw")
        XCTAssertEqual(s.update(), .nothing)
        XCTAssertFalse(s.clockRunning)
    }

    func testALiveStateRunsTheClockAndIdleStopsItWithOneStillFrame() {
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        XCTAssertEqual(s.set(state: .meeting), .startClock)
        XCTAssertTrue(s.clockRunning)
        XCTAssertEqual(s.set(state: .blotting), .nothing, "live to live: the clock keeps running")
        XCTAssertEqual(s.set(state: .idle), .stopClockAndDrawStill)
        XCTAssertFalse(s.clockRunning)
        XCTAssertEqual(s.update(), .nothing)
    }

    func testNothingDrawsOffScreen() {
        var s = InkSchedule()
        XCTAssertEqual(s.set(state: .dictating), .nothing, "live but not on screen")
        XCTAssertEqual(s.set(state: .idle), .nothing)
        XCTAssertEqual(s.invalidate(), .nothing, "a resize off screen draws nothing either")
        XCTAssertEqual(s.set(onScreen: true), .drawStill, "on screen at last: the still frame, once")
    }

    func testHidingALiveInkStopsTheClockAndShowingItStartsItAgain() {
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        _ = s.set(state: .meeting)
        XCTAssertEqual(s.set(onScreen: false), .stopClock, "covered: no frame, not even a still")
        XCTAssertEqual(s.set(onScreen: true), .startClock)
    }

    func testAnIdleInkThatWasCoveredIsNotRedrawnWhenUncovered() {
        // The layer keeps its last frame while covered.
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        _ = s.set(onScreen: false)
        XCTAssertEqual(s.set(onScreen: true), .nothing)
    }

    func testGoingIdleWhileCoveredDrawsTheStillFrameWhenUncovered() {
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        _ = s.set(state: .meeting)
        _ = s.set(onScreen: false)
        XCTAssertEqual(s.set(state: .idle), .nothing)
        XCTAssertEqual(s.set(onScreen: true), .drawStill)
    }

    func testReduceMotionShowsEachStateAsOneStillFrame() {
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        XCTAssertEqual(s.set(reduceMotion: true), .nothing, "idle's still frame is already there")
        XCTAssertEqual(s.set(state: .dictating), .drawStill, "no clock: a still frame of the state")
        XCTAssertEqual(s.set(state: .meeting), .drawStill)
        XCTAssertFalse(s.clockRunning)
        XCTAssertEqual(s.set(reduceMotion: false), .startClock, "motion allowed again")
        XCTAssertEqual(s.set(reduceMotion: true), .stopClockAndDrawStill)
    }

    /// A glide at rest (the main window's orb moving to a new spot) runs the clock for its length
    /// only, then the settled frame at the new spot; never off screen or with motion off.
    func testAGlideAtRestRunsTheClockOnlyWhileItLasts() {
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        XCTAssertEqual(s.set(gliding: true), .startClock)
        XCTAssertEqual(s.set(gliding: false), .stopClockAndDrawStill, "arrived: one still frame there")
        XCTAssertEqual(s.update(), .nothing, "and nothing after it")
        XCTAssertFalse(s.clockRunning)

        _ = s.set(reduceMotion: true)
        XCTAssertEqual(s.set(gliding: true), .nothing, "motion off: no glide")
        _ = s.set(gliding: false)
        _ = s.set(reduceMotion: false)
        _ = s.set(onScreen: false)
        XCTAssertEqual(s.set(gliding: true), .nothing, "off screen: nothing moves")
    }

    func testAResizeWhileIdleRedrawsOnceAndWhileLiveNotAtAll() {
        var s = InkSchedule()
        _ = s.set(onScreen: true)
        XCTAssertEqual(s.invalidate(), .drawStill)
        XCTAssertEqual(s.update(), .nothing)
        _ = s.set(state: .dictating)
        XCTAssertEqual(s.invalidate(), .nothing, "the next live frame draws the new size")
    }
}
