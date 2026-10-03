// When an ink view draws (architecture rule 9: draw nothing when idle). The display link runs
// only while a live state, or a glide at rest (the main window's orb moving to a new spot, for a
// couple of seconds), is on screen and motion is allowed. Every other change draws at most one
// still frame, and only when the frame on screen is out of date; a covered view draws nothing at
// all. No timer here: each decision follows an event (a state change, the window being covered or
// shown, a resize, the Reduce Motion setting, a glide starting or arriving).
import Foundation

/// The view's decisions, kept apart from AppKit so they can be tested one by one.
struct InkSchedule: Equatable {
    /// What the view does after a change.
    enum Action: Equatable {
        case nothing
        /// Settle the ink and draw one frame.
        case drawStill
        /// Start the display link: each tick steps the ink and draws.
        case startClock
        /// Stop the display link. The last live frame stays on the covered layer.
        case stopClock
        /// Stop the display link, then draw the settled frame.
        case stopClockAndDrawStill
    }

    private(set) var state: InkState = .idle
    private(set) var onScreen = false
    private(set) var reduceMotion = false
    private(set) var clockRunning = false
    /// The orb is gliding to a new spot at rest.
    private(set) var gliding = false
    /// The frame on the layer is the settled frame of the current state at the current size.
    private var stillIsCurrent = false

    /// The display link runs only for a live state or a glide, on screen, with motion allowed.
    var wantsClock: Bool { (state.isLive || gliding) && onScreen && !reduceMotion }

    /// A change of state also ends a glide: live, the orb wanders on its own frames; going to
    /// rest, it holds where it got to.
    mutating func set(state new: InkState) -> Action {
        if new != state {
            state = new
            stillIsCurrent = false
            gliding = false
        }
        return update()
    }

    /// Hidden, or with motion stilled, a glide under way ends (the orb holds where it was).
    mutating func set(onScreen new: Bool) -> Action {
        onScreen = new
        if !new { gliding = false }
        return update()
    }

    mutating func set(reduceMotion new: Bool) -> Action {
        reduceMotion = new
        if new { gliding = false }
        return update()
    }

    /// A glide at rest starts (true) or arrives (false).
    mutating func set(gliding new: Bool) -> Action {
        gliding = new
        return update()
    }

    /// The size, the wordmark or the screen changed: the frame on screen is out of date.
    mutating func invalidate() -> Action {
        stillIsCurrent = false
        return update()
    }

    /// The action for the current inputs.
    mutating func update() -> Action {
        if wantsClock {
            stillIsCurrent = false
            if clockRunning { return .nothing }
            clockRunning = true
            return .startClock
        }
        let stopped = clockRunning
        if stopped {
            clockRunning = false
            // The layer holds a live frame, not the settled one.
            stillIsCurrent = false
        }
        if onScreen && !stillIsCurrent {
            stillIsCurrent = true
            return stopped ? .stopClockAndDrawStill : .drawStill
        }
        return stopped ? .stopClock : .nothing
    }
}
