// The icons that show the app's state: the menu-bar mark, and (LiveIcon) what the Dock tile and
// the menu-bar item show over the art while something is live.
import AppKit
import InkBridge
import InkRenderer
import XCTest

@testable import Inkwell

@MainActor
final class StatusGlyphTests: XCTestCase {
    /// A template: macOS tints it for a light or dark menu bar, and for wallpaper tinting.
    func testTheMarkIsATemplateAtBothScales() throws {
        let image = StatusGlyph.image()
        XCTAssertTrue(image.isTemplate)
        XCTAssertEqual(image.size, NSSize(width: 18, height: 18))
        let widths = Set(image.representations.map(\.pixelsWide))
        XCTAssertEqual(widths, [18, 36], "drawn on its own grid at 1x and 2x")
        XCTAssertEqual(image.accessibilityDescription, "Inkwell")
    }

    /// The rim's straight edges and the orb's centre land on whole pixels: every pixel down the
    /// middle column is either fully inked or fully clear, never a soft half-pixel.
    func testTheRimAndTheOrbArePixelAligned() throws {
        for scale in [1, 2] {
            let rep = try XCTUnwrap(
                StatusGlyph.image().representations.first { $0.pixelsWide == 18 * scale } as? NSBitmapImageRep)
            let g = StatusGlyph.geometry(scale: scale)
            let side = 18 * scale
            let middle = side / 2
            func alpha(_ x: Int, _ y: Int) -> CGFloat { rep.colorAt(x: x, y: y)?.alphaComponent ?? -1 }
            for y in 0..<side {
                let rim = (g.inset..<(g.inset + g.stroke)).contains(y)
                    || ((side - g.inset - g.stroke)..<(side - g.inset)).contains(y)
                let orbTop = (side - g.orb) / 2
                let inOrb = (orbTop..<(orbTop + g.orb)).contains(y)
                // The orb's first and last rows are its curve's tip: soft by nature.
                if inOrb && (y == orbTop || y == orbTop + g.orb - 1) { continue }
                let expected: CGFloat = rim || inOrb ? 1 : 0
                XCTAssertEqual(alpha(middle, y), expected, accuracy: 0.001, "\(scale)x, row \(y)")
                XCTAssertEqual(alpha(y, middle), expected, accuracy: 0.001, "\(scale)x, column \(y)")
            }
        }
    }

    /// The same mark at both scales: the rim spans the same points, as a symbol's weights do.
    func testBothScalesDrawTheSameMark() {
        let one = StatusGlyph.geometry(scale: 1)
        let two = StatusGlyph.geometry(scale: 2)
        XCTAssertEqual(CGFloat(one.inset), CGFloat(two.inset) / 2)
        XCTAssertEqual(CGFloat(one.orb), CGFloat(two.orb) / 2)
        XCTAssertEqual(StatusGlyph.rimOuter, NSRect(x: 2, y: 2, width: 14, height: 14))
        XCTAssertEqual(StatusGlyph.orb, NSRect(x: 6, y: 6, width: 6, height: 6))
    }
}

/// Records what a surface was asked to show.
@MainActor
private final class RecordingSurface: LiveIconSurface {
    var shown: [LiveIconFrame] = []
    func show(_ frame: LiveIconFrame) { shown.append(frame) }
}

/// A ticker the test fires by hand.
@MainActor
private final class HandTicker: LiveIconTicker {
    private(set) var starts: [TimeInterval] = []
    private(set) var stops = 0
    private var tick: (@MainActor () -> Void)?
    var running: Bool { tick != nil }

    func start(interval: TimeInterval, _ tick: @escaping @MainActor () -> Void) {
        starts.append(interval)
        self.tick = tick
    }

    func stop() {
        if tick != nil { stops += 1 }
        tick = nil
    }

    func fire(_ times: Int = 1) {
        for _ in 0..<times { tick?() }
    }
}

private let colours = LiveIconColours(
    night: .init(you: GlowColours.RGB(0.4, 0.3, 1), them: GlowColours.RGB(1, 0.6, 0.3)),
    shown: .init(you: GlowColours.RGB(0.3, 0.2, 0.9), them: GlowColours.RGB(0.9, 0.5, 0.2)))

@MainActor
final class LiveIconLookTests: XCTestCase {
    func testEachStateHasItsLook() {
        XCTAssertEqual(LiveIconLook.for(.idle, progress: nil, still: false), .rest)
        XCTAssertEqual(LiveIconLook.for(.dictating, progress: nil, still: false), .glow(.you))
        XCTAssertEqual(LiveIconLook.for(.meeting, progress: nil, still: false), .pulse(.them))
        XCTAssertEqual(LiveIconLook.for(.blotting, progress: 0.5, still: false), .ring(0.5))
        XCTAssertEqual(LiveIconLook.for(.blotting, progress: nil, still: false), .ring(nil), "no number: indeterminate")
        XCTAssertEqual(LiveIconLook.for(.problem, progress: nil, still: false), .glow(.alert))
    }

    /// Always still and Reduce Motion: the recording is a still frame in their colour; nothing
    /// else moved anyway.
    func testStillHoldsTheRecordingStill() {
        XCTAssertEqual(LiveIconLook.for(.meeting, progress: nil, still: true), .glow(.them))
        for state in InkState.allCases {
            XCTAssertFalse(LiveIconLook.for(state, progress: 0.25, still: true).pulses, "\(state)")
        }
        XCTAssertEqual(InkState.allCases.filter { LiveIconLook.for($0, progress: nil, still: false).pulses }, [.meeting])
    }

    /// The ring fills with the steps the final pass has reported (the ones Today lists), out of
    /// the four it can report; before the first, there is no number.
    func testTheFinalPassProgressIsTheStepsReported() {
        var meeting = CoreStore.LiveMeeting(record: "r1")
        XCTAssertNil(LiveIcon.finalPassProgress(nil))
        XCTAssertNil(LiveIcon.finalPassProgress(meeting))
        meeting.transcribed = [.mic]
        XCTAssertEqual(LiveIcon.finalPassProgress(meeting), 0.25)
        meeting.transcribed = [.mic, .far]
        XCTAssertEqual(LiveIcon.finalPassProgress(meeting), 0.5)
        meeting.diarized = true
        XCTAssertEqual(LiveIcon.finalPassProgress(meeting), 0.75)
        meeting.summarized = true
        XCTAssertEqual(LiveIcon.finalPassProgress(meeting), 1)

        // A later step comes after both sides, as Today reads it.
        var skipped = CoreStore.LiveMeeting(record: "r2")
        skipped.summarized = true
        XCTAssertEqual(LiveIcon.finalPassProgress(skipped), 0.75)
    }
}

@MainActor
final class LiveIconPulseTests: XCTestCase {
    func testNoTimerAtRestAndNothingRedrawsAfterSettling() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let surface = RecordingSurface()
        icon.attach(surface)
        XCTAssertEqual(surface.shown.map(\.look), [.rest], "an attached surface is told once what to show")
        icon.update(look: .rest, colours: colours)
        icon.update(look: .rest, colours: colours)
        XCTAssertEqual(surface.shown.count, 1, "the same look again draws nothing")
        XCTAssertTrue(ticker.starts.isEmpty)
        XCTAssertFalse(icon.isPulsing)
    }

    /// Dictating is a still frame: drawn when the state or a colour changes, and never between.
    func testAStillLookRedrawsOnlyOnChange() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let surface = RecordingSurface()
        icon.attach(surface)
        icon.update(look: .glow(.you), colours: colours)
        icon.update(look: .glow(.you), colours: colours)
        XCTAssertEqual(surface.shown.count, 2)
        var recoloured = colours
        recoloured.shown.you = GlowColours.RGB(0, 1, 0)
        icon.update(look: .glow(.you), colours: recoloured)
        XCTAssertEqual(surface.shown.count, 3, "a new colour is one frame")
        XCTAssertEqual(surface.shown.last?.strength, 1)
        XCTAssertTrue(ticker.starts.isEmpty, "no timer for a still look")
    }

    func testThePulseStartsAndStopsWithTheRecording() throws {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let surface = RecordingSurface()
        icon.attach(surface)
        icon.update(look: .pulse(.them), colours: colours)
        XCTAssertTrue(icon.isPulsing)
        XCTAssertEqual(ticker.starts.count, 1)
        let interval = try XCTUnwrap(ticker.starts.first)
        XCTAssertEqual(1 / interval, LiveIcon.pulseFPS, accuracy: 0.001)
        XCTAssertEqual(surface.shown.last?.strength, 1, "the recording shows at once, at full strength")

        // The same look again (a colour or progress echo) does not restart the cycle.
        icon.update(look: .pulse(.them), colours: colours)
        XCTAssertEqual(ticker.starts.count, 1)

        let before = surface.shown.count
        ticker.fire(Int(LiveIcon.pulseFPS * LiveIcon.breathPeriod))
        XCTAssertEqual(surface.shown.count, before + 14, "one frame a tick: 7 fps")
        let strengths = surface.shown.suffix(14).map(\.strength)
        XCTAssertLessThan(try XCTUnwrap(strengths.min()), 0.7, "it breathes out")
        XCTAssertEqual(try XCTUnwrap(strengths.last), 1, accuracy: 0.001, "and back in, every two seconds")

        icon.update(look: .ring(nil), colours: colours)
        XCTAssertFalse(icon.isPulsing, "stopped the moment recording ends")
        XCTAssertEqual(ticker.stops, 1)
        XCTAssertEqual(surface.shown.last, LiveIconFrame(look: .ring(nil), colours: colours, strength: 1))
    }

    func testStillMeansNoTimer() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        icon.attach(RecordingSurface())
        icon.update(look: LiveIconLook.for(.meeting, progress: nil, still: true), colours: colours)
        XCTAssertFalse(icon.isPulsing)
        XCTAssertTrue(ticker.starts.isEmpty)
    }

    /// With nothing to draw on, nothing ticks; the pulse resumes on the next surface.
    func testNoSurfaceMeansNoTimer() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        icon.update(look: .pulse(.them), colours: colours)
        XCTAssertFalse(icon.isPulsing)
        let surface = RecordingSurface()
        icon.attach(surface)
        XCTAssertTrue(icon.isPulsing)
        icon.detach(surface)
        XCTAssertFalse(icon.isPulsing)
        ticker.fire()
        XCTAssertEqual(surface.shown.count, 1, "a detached surface is drawn no more")
    }

    /// Frames per minute, on a simulated clock: 0 at rest, 1 for a dictation that starts, 420
    /// for a minute's recording (7 fps).
    func testFramesPerMinute() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        icon.attach(RecordingSurface())
        let minute = Int(LiveIcon.pulseFPS * 60)

        var start = icon.frames
        icon.update(look: .rest, colours: colours)
        ticker.fire(minute)
        XCTAssertEqual(icon.frames - start, 0, "idle")

        start = icon.frames
        icon.update(look: .glow(.you), colours: colours)
        for _ in 0..<minute { icon.update(look: .glow(.you), colours: colours) }
        ticker.fire(minute)
        XCTAssertEqual(icon.frames - start, 1, "dictating: the one change")

        icon.update(look: .pulse(.them), colours: colours)
        start = icon.frames
        ticker.fire(minute)
        XCTAssertEqual(icon.frames - start, 420, "recording")
    }
}

/// The energy probe: the app's own timer on the main run loop, counted over real seconds.
@MainActor
final class LiveIconEnergyTests: XCTestCase {
    func testRedrawsOverRealTime() {
        let icon = LiveIcon()
        let surface = RecordingSurface()
        icon.attach(surface)
        func frames(over seconds: Double) -> Int {
            let start = icon.frames
            RunLoop.main.run(until: Date().addingTimeInterval(seconds))
            return icon.frames - start
        }

        icon.update(look: .rest, colours: colours)
        let idle = frames(over: 1.5)
        XCTAssertEqual(idle, 0, "idle")

        icon.update(look: .glow(.you), colours: colours)
        let dictating = frames(over: 1.5)
        XCTAssertEqual(dictating, 0, "dictating, after its one change")

        icon.update(look: .pulse(.them), colours: colours)
        let recording = frames(over: 3)
        XCTAssertLessThanOrEqual(recording, 22, "no faster than 7 fps")
        XCTAssertGreaterThanOrEqual(recording, 15, "and close to it")

        icon.update(look: .ring(0.5), colours: colours)
        XCTAssertFalse(icon.isPulsing)
        let after = frames(over: 1)
        XCTAssertEqual(after, 0, "the pulse stopped with the recording")
        print("live icon frames per minute: idle \(idle * 40), dictating \(dictating * 40) after its change, "
            + "recording \(recording * 20), after recording \(after * 60)")
    }
}
