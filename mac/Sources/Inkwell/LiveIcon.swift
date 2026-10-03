// The icons that show what the app is doing: the Dock tile (while the main window is open) and the
// menu-bar item. Driven by state, never by a clock:
//
//   idle        the icon as it is; nothing redraws
//   dictating   the orb in your colour, a still frame
//   recording   the orb in their colour, breathing at 7 fps on a two-second cycle; a still frame
//               in their colour under Always still or Reduce Motion
//   final pass  a ring that fills with the steps the final pass has reported; indeterminate
//               until the first
//   a problem   the orb in the alert colour
//
// LiveIcon decides what each surface shows and when it redraws. A still look is drawn once, when
// it or its colours change. The recording's pulse is breathed two ways: the menu-bar item runs it
// as a layer animation (the render server's, at the pulse's low rate, with no wakeup in the app),
// and the Dock tile, which has no layers to animate, is ticked by a timer that runs only while the
// tile is there and a pulse is shown. Each surface draws the frames it is given.
import Foundation
import InkBridge
import InkRenderer
import Observation

/// What the icon shows.
enum LiveIconLook: Equatable, Sendable {
    /// Whose colour a look takes.
    enum Tone: Equatable, Sendable {
        case you
        case them
        case alert
    }

    /// The icon as it is.
    case rest
    /// The orb in a colour, still.
    case glow(Tone)
    /// The orb in a colour, breathing.
    case pulse(Tone)
    /// A ring around the plate, filled to the final pass's progress (0...1), or indeterminate.
    case ring(Double?)

    /// The look for `state`. `progress`: the final pass's, when there is a number. `still`: Always
    /// still or Reduce Motion, which hold the recording still in its colour.
    static func `for`(_ state: InkState, progress: Double?, still: Bool) -> LiveIconLook {
        switch state {
        case .idle: .rest
        case .dictating: .glow(.you)
        case .meeting: still ? .glow(.them) : .pulse(.them)
        case .blotting: .ring(progress)
        case .problem: .glow(.alert)
        }
    }

    /// Whether it moves (the only look that runs a timer).
    var pulses: Bool {
        if case .pulse = self { true } else { false }
    }
}

/// The user's dot colours, for each surface's background.
struct LiveIconColours: Equatable, Sendable {
    struct Pair: Equatable, Sendable {
        var you: GlowColours.RGB
        var them: GlowColours.RGB

        /// The colour of `tone`; the alert colour is the surface's own.
        func colour(_ tone: LiveIconLook.Tone, alert: GlowColours.RGB) -> GlowColours.RGB {
            switch tone {
            case .you: you
            case .them: them
            case .alert: alert
            }
        }
    }

    /// For the Dock tile: the icon's plate is night in either mode, so it takes the colours the
    /// user chose for the dark mode, which are fitted to read on night.
    var night: Pair
    /// For the menu bar: the mode the app shows, as every other dot.
    var shown: Pair

    static let unset = LiveIconColours(
        night: Pair(you: .zero, them: .zero), shown: Pair(you: .zero, them: .zero))
}

/// One frame of the icon.
struct LiveIconFrame: Equatable, Sendable {
    var look: LiveIconLook
    var colours: LiveIconColours
    /// How strongly the colour shows, 0...1: 1 for a still look; the pulse breathes it.
    var strength: Double
}

/// Something that shows the icon: the Dock tile, the menu-bar item.
@MainActor
protocol LiveIconSurface: AnyObject {
    /// It breathes the pulse itself (a layer animation), so it is shown the pulse once and never
    /// ticked.
    var breathesItself: Bool { get }
    func show(_ frame: LiveIconFrame)
}

extension LiveIconSurface {
    var breathesItself: Bool { false }
}

/// The pulse's clock.
@MainActor
protocol LiveIconTicker: AnyObject {
    var running: Bool { get }
    /// Calls `tick` every `interval` seconds, on the main thread, until stopped.
    func start(interval: TimeInterval, _ tick: @escaping @MainActor () -> Void)
    func stop()
}

@MainActor
final class LiveIcon {
    /// The pulse's frame rate: a breath reads as smooth at 7 fps, at a fraction of a display's
    /// 60 or 120 Hz.
    static let pulseFPS = 7.0
    /// One breath, out and in, in seconds.
    static let breathPeriod = 2.0
    /// The faintest the pulse gets: it dims, never goes out, so the recording always shows.
    static let breathLow = 0.55

    private(set) var frame = LiveIconFrame(look: .rest, colours: .unset, strength: 1)
    /// Frames the app drew since launch: each change, and each tick of the pulse. The energy
    /// budget's count (0 a minute at rest, 1 a change while dictating; while recording, 420 a
    /// minute with the Dock tile showing, 1 with only the menu bar, which breathes by itself).
    private(set) var frames = 0

    private let ticker: LiveIconTicker
    private var surfaces: [LiveIconSurface] = []
    /// Ticks into the current breath.
    private var tick = 0

    init(ticker: LiveIconTicker = TimerTicker()) {
        self.ticker = ticker
    }

    var isPulsing: Bool { ticker.running }

    /// Shows `surface` the current frame, and every frame from now on.
    func attach(_ surface: LiveIconSurface) {
        guard !surfaces.contains(where: { $0 === surface }) else { return }
        surfaces.append(surface)
        surface.show(frame)
        reconcileTicker()
    }

    func detach(_ surface: LiveIconSurface) {
        surfaces.removeAll { $0 === surface }
        reconcileTicker()
    }

    /// The look and colours now. The same again draws nothing; a pulse already running keeps its
    /// place in the breath.
    func update(look: LiveIconLook, colours: LiveIconColours) {
        // At rest the icon shows no colour: a theme change (or the first colours at launch) is
        // kept for later and draws nothing.
        if look == .rest && frame.look == .rest {
            frame.colours = colours
            return
        }
        var next = frame
        next.look = look
        next.colours = colours
        if look != frame.look {
            tick = 0
            next.strength = 1
        }
        guard next != frame else { return }
        deliver(next)
        reconcileTicker()
    }

    /// The final pass's progress, 0...1, from the steps the core has reported (the ones Today
    /// lists): each side transcribed, the far end's speakers told apart, the summary written. A
    /// step the core skips is never reported, so the ring may not reach full before the pass
    /// ends; it never claims a step that has not happened. Nil (indeterminate) before the first.
    nonisolated static func finalPassProgress(_ meeting: CoreStore.LiveMeeting?) -> Double? {
        guard let meeting else { return nil }
        // A later step comes after both sides (LiveCard.blottingLine reads it so too).
        let later = meeting.diarized || meeting.summarized
        let sides = later ? 2 : min(meeting.transcribed.count, 2)
        let steps = sides + (meeting.diarized ? 1 : 0) + (meeting.summarized ? 1 : 0)
        return steps == 0 ? nil : Double(steps) / 4
    }

    /// The theme's colours for each surface.
    static func colours(_ theme: GlowTheme) -> LiveIconColours {
        let shown = theme.dots
        let night = theme.nightDots
        return LiveIconColours(
            night: .init(you: night.you, them: night.them), shown: .init(you: shown.you, them: shown.them))
    }

    /// Follows the ink's state, the final pass's steps, the theme's colours and the motion
    /// settings through observation, from now on: nothing polls, and a change that alters nothing
    /// shown draws nothing (update).
    func follow(ink: ShellInk, theme: GlowTheme) {
        withObservationTracking {
            let state = ink.state
            let progress = state == .blotting ? Self.finalPassProgress(ink.store.meeting) : nil
            let still = theme.motionStill || theme.reduceMotion
            update(look: .for(state, progress: progress, still: still), colours: Self.colours(theme))
        } onChange: { [weak self] in
            // Read on the next turn of the main queue, once the change is applied.
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.follow(ink: ink, theme: theme) }
            }
        }
    }

    /// The strength `tick` frames into a breath: full, out to `breathLow`, and back.
    static func breath(_ tick: Int) -> Double {
        let phase = Double(tick) / (pulseFPS * breathPeriod)
        return breathLow + (1 - breathLow) * (0.5 + 0.5 * cos(2 * .pi * phase))
    }

    private func deliver(_ next: LiveIconFrame) {
        frame = next
        frames += 1
        for surface in surfaces {
            surface.show(next)
        }
    }

    /// The surfaces the pulse's timer draws on.
    private var ticked: [LiveIconSurface] { surfaces.filter { !$0.breathesItself } }

    /// Runs the timer exactly while a pulse is shown on a surface that needs its frames.
    private func reconcileTicker() {
        let wanted = frame.look.pulses && !ticked.isEmpty
        if wanted && !ticker.running {
            ticker.start(interval: 1 / Self.pulseFPS) { [weak self] in self?.advance() }
        } else if !wanted && ticker.running {
            ticker.stop()
        }
    }

    private func advance() {
        guard frame.look.pulses else { return }
        tick = (tick + 1) % Int(Self.pulseFPS * Self.breathPeriod)
        frame.strength = Self.breath(tick)
        frames += 1
        for surface in ticked {
            surface.show(frame)
        }
    }
}

/// The pulse's clock in the app: a run-loop timer, in the common modes so the pulse holds while a
/// menu is open.
@MainActor
final class TimerTicker: LiveIconTicker {
    private var timer: Timer?

    var running: Bool { timer != nil }

    func start(interval: TimeInterval, _ tick: @escaping @MainActor () -> Void) {
        stop()
        let timer = Timer(timeInterval: interval, repeats: true) { _ in
            MainActor.assumeIsolated { tick() }
        }
        // A fifth of a frame's slack lets the system coalesce the wakeup with others'.
        timer.tolerance = interval / 5
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }
}
