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

    /// While the overlay shows a coloured orb, the mark has none of its own: the breath then fades
    /// the colour toward the menu bar, never toward the template's dot (which read as brown).
    func testTheMarkGoesHollowUnderAColouredOrb() throws {
        XCTAssertTrue(StatusGlyph.markHasOrb(under: .rest))
        XCTAssertTrue(StatusGlyph.markHasOrb(under: .ring(0.5)), "the ring leaves the orb as it is")
        XCTAssertFalse(StatusGlyph.markHasOrb(under: .glow(.you)))
        XCTAssertFalse(StatusGlyph.markHasOrb(under: .pulse(.them)))
        XCTAssertFalse(StatusGlyph.markHasOrb(under: .glow(.alert)))

        let hollow = StatusGlyph.image(orb: false)
        XCTAssertTrue(hollow.isTemplate)
        XCTAssertEqual(Set(hollow.representations.map(\.pixelsWide)), [18, 36])
        for scale in [1, 2] {
            let rep = try XCTUnwrap(hollow.representations.first { $0.pixelsWide == 18 * scale } as? NSBitmapImageRep)
            let side = 18 * scale
            let g = StatusGlyph.geometry(scale: scale)
            XCTAssertEqual(rep.colorAt(x: side / 2, y: side / 2)?.alphaComponent ?? -1, 0, accuracy: 0.001, "\(scale)x: no orb")
            XCTAssertEqual(rep.colorAt(x: side / 2, y: g.inset)?.alphaComponent ?? -1, 1, accuracy: 0.001, "\(scale)x: the rim")
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
    let breathesItself: Bool
    var shown: [LiveIconFrame] = []
    var awake: [Bool] = []
    init(breathesItself: Bool = false) { self.breathesItself = breathesItself }
    func show(_ frame: LiveIconFrame) { shown.append(frame) }
    func setAwake(_ awake: Bool) { self.awake.append(awake) }
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

    /// The last closure, kept past stop(): a tick already on its way when the timer stopped.
    private var last: (@MainActor () -> Void)?

    func stop() {
        if tick != nil { stops += 1 }
        last = tick
        tick = nil
    }

    func fire(_ times: Int = 1) {
        for _ in 0..<times { tick?() }
    }

    /// Fires the stopped timer's closure, as a late tick would.
    func fireLate(_ times: Int = 1) {
        for _ in 0..<times { (tick ?? last)?() }
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

    /// A tick already on its way when the recording ended draws nothing.
    func testALateTickDrawsNothing() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let surface = RecordingSurface()
        icon.attach(surface)
        icon.update(look: .pulse(.them), colours: colours)
        icon.update(look: .glow(.you), colours: colours)
        let shown = surface.shown.count
        ticker.fireLate(3)
        XCTAssertEqual(surface.shown.count, shown)
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

@MainActor
final class LiveIconSelfBreathTests: XCTestCase {
    /// The menu bar breathes by itself (a layer animation run by the render server): it is told
    /// the pulse once, and no timer runs for it.
    func testASurfaceThatBreathesItselfRunsNoTimer() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let bar = RecordingSurface(breathesItself: true)
        icon.attach(bar)
        icon.update(look: .pulse(.them), colours: colours)
        XCTAssertFalse(icon.isPulsing)
        XCTAssertTrue(ticker.starts.isEmpty)
        XCTAssertEqual(bar.shown.last, LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))

        // The Dock tile needs the frames: the timer runs while it is there, and only it is ticked.
        let dock = RecordingSurface()
        icon.attach(dock)
        XCTAssertTrue(icon.isPulsing)
        let barFrames = bar.shown.count
        ticker.fire(14)
        XCTAssertEqual(bar.shown.count, barFrames, "the menu bar is not ticked")
        XCTAssertEqual(dock.shown.count, 15)
        icon.detach(dock)
        XCTAssertFalse(icon.isPulsing, "the window closed: nothing left to tick")
        XCTAssertEqual(ticker.stops, 1)
    }

    /// A minute's recording with the window closed: one frame (the change) for the app to draw.
    func testAMinuteRecordingInTheMenuBarAloneIsOneFrame() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        icon.attach(RecordingSurface(breathesItself: true))
        let start = icon.frames
        icon.update(look: .pulse(.them), colours: colours)
        XCTAssertTrue(ticker.starts.isEmpty, "no timer at all")
        XCTAssertEqual(icon.frames - start, 1)
    }
}

/// Nobody can see the screen (the displays asleep, the screen locked, another user in front):
/// nothing ticks, nothing breathes, nothing is drawn, until it can be seen again.
@MainActor
final class LiveIconAsleepTests: XCTestCase {
    func testAsleepNothingTicksOrDraws() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let dock = RecordingSurface()
        let bar = RecordingSurface(breathesItself: true)
        icon.attach(dock)
        icon.attach(bar)
        icon.update(look: .pulse(.them), colours: colours)
        XCTAssertTrue(icon.isPulsing)

        icon.setAwake(false)
        XCTAssertFalse(icon.isPulsing, "the Dock's timer stops")
        XCTAssertEqual(bar.awake, [false], "the menu bar's breath stops")
        let start = icon.frames
        let shown = dock.shown.count + bar.shown.count
        ticker.fireLate(420)
        icon.update(look: .ring(0.25), colours: colours)
        icon.update(look: .ring(0.5), colours: colours)
        XCTAssertEqual(icon.frames - start, 0, "a minute asleep, with the state changing: nothing drawn")
        XCTAssertEqual(dock.shown.count + bar.shown.count, shown)

        icon.setAwake(true)
        XCTAssertEqual(bar.awake, [false, true])
        XCTAssertEqual(dock.shown.last?.look, .ring(0.5), "awake: the state as it is now, at once")
        XCTAssertEqual(bar.shown.last?.look, .ring(0.5))
        XCTAssertEqual(icon.frames - start, 1)
        XCTAssertFalse(icon.isPulsing, "the recording ended while asleep")
    }

    /// Awake again mid-recording: the breath picks up, with nothing to redraw.
    func testTheBreathResumesOnWake() {
        let ticker = HandTicker()
        let icon = LiveIcon(ticker: ticker)
        let dock = RecordingSurface()
        icon.attach(dock)
        icon.update(look: .pulse(.them), colours: colours)
        icon.setAwake(false)
        icon.setAwake(false)
        let shown = dock.shown.count
        icon.setAwake(true)
        XCTAssertTrue(icon.isPulsing)
        XCTAssertEqual(ticker.starts.count, 2)
        XCTAssertEqual(dock.shown.count, shown, "nothing changed while asleep")
        ticker.fire()
        XCTAssertEqual(dock.shown.count, shown + 1)
    }

    /// A surface attached while nobody can see is told so before it is shown anything.
    func testASurfaceAttachedAsleepStartsAsleep() {
        let icon = LiveIcon(ticker: HandTicker())
        icon.update(look: .pulse(.them), colours: colours)
        icon.setAwake(false)
        let bar = RecordingSurface(breathesItself: true)
        icon.attach(bar)
        XCTAssertEqual(bar.awake, [false])
        XCTAssertTrue(bar.shown.isEmpty, "nothing drawn on a sleeping display")
        XCTAssertFalse(icon.isPulsing)
        icon.setAwake(true)
        XCTAssertEqual(bar.shown.map(\.look), [.pulse(.them)], "shown on waking")
    }
}

@MainActor
final class DisplayWatchTests: XCTestCase {
    func testItFollowsSleepLockAndTheSession() {
        let workspace = NotificationCenter()
        let app = NotificationCenter()
        let distributed = NotificationCenter()
        var said: [Bool] = []
        let watch = DisplayWatch(workspace: workspace, app: app, distributed: distributed) { said.append($0) }
        XCTAssertTrue(watch.awake)
        workspace.post(name: NSWorkspace.screensDidSleepNotification, object: nil)
        XCTAssertEqual(said, [false])
        distributed.post(name: DisplayWatch.screenLocked, object: nil)
        workspace.post(name: NSWorkspace.screensDidWakeNotification, object: nil)
        XCTAssertEqual(said, [false], "awake displays, but locked")
        distributed.post(name: DisplayWatch.screenUnlocked, object: nil)
        XCTAssertEqual(said, [false, true])
        workspace.post(name: NSWorkspace.sessionDidResignActiveNotification, object: nil)
        workspace.post(name: NSWorkspace.sessionDidBecomeActiveNotification, object: nil)
        XCTAssertEqual(said, [false, true, false, true], "another user in front, then back")
    }

    /// A missed unlock or wake never leaves the icon still for good: the app becoming active
    /// means someone is at an awake, unlocked screen.
    func testItFailsOpenWhenTheAppIsActive() {
        let workspace = NotificationCenter()
        let app = NotificationCenter()
        let distributed = NotificationCenter()
        var said: [Bool] = []
        let watch = DisplayWatch(workspace: workspace, app: app, distributed: distributed) { said.append($0) }
        distributed.post(name: DisplayWatch.screenLocked, object: nil)
        workspace.post(name: NSWorkspace.screensDidSleepNotification, object: nil)
        XCTAssertFalse(watch.awake)
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        XCTAssertTrue(watch.awake)
        XCTAssertEqual(said, [false, true])
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
        // Loose: a busy machine (or the thread sanitizer) delays a timer, never hastens it.
        XCTAssertGreaterThanOrEqual(recording, 8, "and ticking")

        icon.setAwake(false)
        let asleep = frames(over: 1.5)
        XCTAssertEqual(asleep, 0, "recording, the display asleep")
        icon.setAwake(true)

        icon.update(look: .ring(0.5), colours: colours)
        XCTAssertFalse(icon.isPulsing)
        let after = frames(over: 1)
        XCTAssertEqual(after, 0, "the pulse stopped with the recording")
        print("live icon frames per minute (Dock tile): idle \(idle * 40), dictating \(dictating * 40) after its change, "
            + "recording \(recording * 20), recording with the display asleep \(asleep * 40), after recording \(after * 60)")
    }
}

/// A Dock tile that counts what it is asked to do.
@MainActor
private final class CountingTile: LiveIconTile {
    var contentView: NSView?
    var size = NSSize(width: 128, height: 128)
    var displays = 0
    func display() { displays += 1 }
}

/// The app icon as the bundle carries it.
@MainActor
private func appIcon() throws -> NSImage {
    let mac = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    return try XCTUnwrap(NSImage(contentsOf: mac.appendingPathComponent("AppIcon.icns")))
}

/// `frame` drawn over the icon, `side` pixels square.
@MainActor
private func render(_ frame: LiveIconFrame?, side: Int = 256) throws -> NSBitmapImageRep {
    let rep = try XCTUnwrap(NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
    let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: rep))
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    let rect = NSRect(x: 0, y: 0, width: side, height: side)
    let base = try appIcon()
    if let frame {
        LiveIconArt.draw(frame, base: base, in: rect)
    } else {
        base.draw(in: rect)
    }
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

private func rgb(_ rep: NSBitmapImageRep, _ x: CGFloat, _ y: CGFloat) -> GlowColours.RGB {
    // colorAt counts rows from the top; the art's points count from the bottom.
    let c = rep.colorAt(x: Int(x), y: rep.pixelsHigh - 1 - Int(y))?.usingColorSpace(.deviceRGB)
    return GlowColours.RGB(Double(c?.redComponent ?? -1), Double(c?.greenComponent ?? -1), Double(c?.blueComponent ?? -1))
}

private func distance(_ a: GlowColours.RGB, _ b: GlowColours.RGB) -> Double {
    let d = a - b
    return (d * d).sum().squareRoot()
}

@MainActor
final class LiveIconDockTests: XCTestCase {
    /// At rest the tile has no view of its own: macOS draws the bundle's icon, exactly the static
    /// one. Live, the view draws the frame, once per frame.
    func testTheTileDrawsOnlyWhileLive() {
        let tile = CountingTile()
        let dock = LiveIconDock(tile: tile, base: NSImage(size: NSSize(width: 16, height: 16)))
        dock.show(LiveIconFrame(look: .rest, colours: colours, strength: 1))
        XCTAssertNil(tile.contentView)
        XCTAssertEqual(tile.displays, 0, "at rest from the start: nothing to put back")

        dock.show(LiveIconFrame(look: .glow(.you), colours: colours, strength: 1))
        XCTAssertNotNil(tile.contentView)
        XCTAssertEqual(tile.contentView?.frame.size, tile.size)
        XCTAssertEqual(tile.displays, 1)
        dock.show(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 0.8))
        XCTAssertEqual(tile.displays, 2)

        dock.show(LiveIconFrame(look: .rest, colours: colours, strength: 1))
        XCTAssertNil(tile.contentView, "back to the bundle's icon")
        XCTAssertEqual(tile.displays, 3)

        // A window closed while live leaves no view behind for the next tile.
        dock.show(LiveIconFrame(look: .ring(nil), colours: colours, strength: 1))
        dock.clear()
        XCTAssertNil(tile.contentView)
    }

    /// Each look over the icon: the orb takes the colour, the ring fills clockwise from the top,
    /// and the plate's corners stay the icon's own.
    func testTheArtShowsEachLook() throws {
        let side: CGFloat = 256
        let orb = CGPoint(x: side / 2, y: side * LiveIconArt.orbCentre.y)
        let still = try render(nil)
        let glow = try render(LiveIconFrame(look: .glow(.you), colours: colours, strength: 1))
        XCTAssertLessThan(distance(rgb(glow, orb.x + 20, orb.y), colours.night.you), 0.15, "the orb in your night colour")
        XCTAssertGreaterThan(distance(rgb(still, orb.x + 20, orb.y), colours.night.you), 0.2, "which the icon is not")
        let corner = CGPoint(x: side * 0.16, y: side * 0.16)
        XCTAssertLessThan(distance(rgb(glow, corner.x, corner.y), rgb(still, corner.x, corner.y)), 0.02)

        let alert = try render(LiveIconFrame(look: .glow(.alert), colours: colours, strength: 1))
        XCTAssertLessThan(distance(rgb(alert, orb.x + 20, orb.y), LiveIconArt.alert), 0.15)

        let faint = try render(LiveIconFrame(look: .pulse(.them), colours: colours, strength: LiveIcon.breathLow))
        let full = try render(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))
        XCTAssertLessThan(distance(rgb(full, orb.x + 20, orb.y), colours.night.them), 0.15)
        XCTAssertGreaterThan(distance(rgb(faint, orb.x + 20, orb.y), rgb(full, orb.x + 20, orb.y)), 0.05, "it breathes")

        // Half way: the right side is filled, the left is the track.
        let half = try render(LiveIconFrame(look: .ring(0.5), colours: colours, strength: 1))
        let ring = LiveIconArt.ringRect(in: NSRect(x: 0, y: 0, width: side, height: side))
        let right = CGPoint(x: ring.maxX, y: ring.midY)
        let left = CGPoint(x: ring.minX, y: ring.midY)
        XCTAssertLessThan(distance(rgb(half, right.x, right.y), colours.night.them), 0.15, "filled")
        XCTAssertGreaterThan(distance(rgb(half, left.x, left.y), colours.night.them), 0.3, "still to come")
        XCTAssertLessThan(distance(rgb(half, orb.x + 20, orb.y), rgb(still, orb.x + 20, orb.y)), 0.02,
                          "the orb is left as it is")
    }
}

@MainActor
final class LiveIconFeedTests: XCTestCase {
    /// The theme's colours: the shown mode's for the menu bar, the dark mode's for the Dock.
    func testTheColoursComeFromTheTheme() {
        let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
        theme.setMode(.light)
        theme.setYou("#336699")
        theme.setMode(.dark)
        theme.setYou("#aa33ff")
        theme.setMode(.light)
        let c = LiveIcon.colours(theme)
        XCTAssertEqual(GlowColours.hex(c.shown.you), GlowColours.hex(theme.dots.you))
        XCTAssertEqual(GlowColours.hex(c.night.you), "#aa33ff", "the Dock's plate is night in either mode")
    }

    /// The look follows the ink, the final pass's steps and the motion setting, without polling.
    func testTheIconFollowsTheInk() async throws {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
        let icon = LiveIcon(ticker: HandTicker())
        icon.follow(ink: ink, theme: theme)
        XCTAssertEqual(icon.frame.look, .rest)

        ink.held = .meeting
        await Task.yield()
        try await waitFor { icon.frame.look == .pulse(.them) }
        theme.setMotion(.still)
        try await waitFor { icon.frame.look == .glow(.them) }
        ink.held = nil
        try await waitFor { icon.frame.look == .rest }
    }

    private func waitFor(_ condition: @MainActor () -> Bool) async throws {
        for _ in 0..<100 where !condition() {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(condition())
    }
}

/// Renders for a design review, written only when asked:
///
///   INK_LIVE_ICON_RENDER=<folder>   the Dock tile and the menu-bar item in each state, with the
///                                   default colours (the menu bar light and dark, 2x)
@MainActor
final class LiveIconRenderTests: XCTestCase {
    func testRenderTheDockTile() throws {
        guard let folder = ProcessInfo.processInfo.environment["INK_LIVE_ICON_RENDER"] else {
            throw XCTSkip("INK_LIVE_ICON_RENDER is not set")
        }
        let out = URL(fileURLWithPath: folder, isDirectory: true)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let colours = LiveIcon.colours(GlowTheme(send: { _ in }, applyAppearance: { _ in }))
        let frames: [(String, LiveIconFrame?)] = [
            ("idle", nil),
            ("dictating", LiveIconFrame(look: .glow(.you), colours: colours, strength: 1)),
            ("recording-breath-in", LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1)),
            ("recording-breath-out", LiveIconFrame(look: .pulse(.them), colours: colours, strength: LiveIcon.breathLow)),
            ("recording-still", LiveIconFrame(look: .glow(.them), colours: colours, strength: 1)),
            ("final-pass-indeterminate", LiveIconFrame(look: .ring(nil), colours: colours, strength: 1)),
            ("final-pass-50", LiveIconFrame(look: .ring(0.5), colours: colours, strength: 1)),
            ("final-pass-75", LiveIconFrame(look: .ring(0.75), colours: colours, strength: 1)),
            ("problem", LiveIconFrame(look: .glow(.alert), colours: colours, strength: 1)),
        ]
        for (name, frame) in frames {
            let png = try XCTUnwrap(render(frame, side: 512).representation(using: .png, properties: [:]))
            try png.write(to: out.appendingPathComponent("dock-\(name).png"))
        }
    }

    func testRenderTheMenuBarItem() throws {
        guard let folder = ProcessInfo.processInfo.environment["INK_LIVE_ICON_RENDER"] else {
            throw XCTSkip("INK_LIVE_ICON_RENDER is not set")
        }
        let out = URL(fileURLWithPath: folder, isDirectory: true)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let looks: [(String, LiveIconLook, Double)] = [
            ("idle", .rest, 1), ("dictating", .glow(.you), 1), ("recording-breath-in", .pulse(.them), 1),
            ("recording-mid-breath", .pulse(.them), 0.5), ("recording-breath-out", .pulse(.them), 0),
            ("final-pass-indeterminate", .ring(nil), 1),
            ("final-pass-50", .ring(0.5), 1), ("problem", .glow(.alert), 1),
        ]
        // An item's cell on a 24 pt menu bar, at 2x. The menu bar's own material is approximated
        // by a flat fill, and the template's tint by the label colour, as macOS draws it.
        let cell = NSSize(width: 24, height: 24)
        for (bar, name) in [(NSAppearance.Name.aqua, "light"), (.darkAqua, "dark")] {
            let appearance = try XCTUnwrap(NSAppearance(named: bar))
            let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
            theme.setMode(bar == .darkAqua ? .dark : .light)
            let colours = LiveIcon.colours(theme)
            let strip = try XCTUnwrap(NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: Int(cell.width) * 2 * looks.count, pixelsHigh: Int(cell.height) * 2,
                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                bytesPerRow: 0, bitsPerPixel: 0))
            strip.size = NSSize(width: cell.width * CGFloat(looks.count), height: cell.height)
            let window = NSWindow(
                contentRect: NSRect(origin: .zero, size: cell), styleMask: .borderless, backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            defer { window.close() }
            window.appearance = appearance
            let overlay = StatusGlyphOverlay(frame: NSRect(origin: .zero, size: cell))
            window.contentView?.addSubview(overlay)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: strip)
            appearance.performAsCurrentDrawingAppearance {
                (bar == .darkAqua ? NSColor(white: 0.16, alpha: 1) : NSColor(white: 0.93, alpha: 1)).setFill()
                NSRect(origin: .zero, size: strip.size).fill()
                func tinted(_ mark: NSImage) -> NSImage {
                    NSImage(size: mark.size, flipped: false) { rect in
                        mark.draw(in: rect)
                        NSColor.labelColor.setFill()
                        rect.fill(using: .sourceAtop)
                        return true
                    }
                }
                for (i, look) in looks.enumerated() {
                    let origin = CGPoint(x: CGFloat(i) * cell.width, y: 0)
                    let tinted = tinted(StatusGlyph.image(orb: StatusGlyph.markHasOrb(under: look.1)))
                    tinted.draw(in: StatusGlyphOverlay.glyphRect(in: NSRect(origin: .zero, size: cell)).offsetBy(dx: origin.x, dy: 0))
                    overlay.show(LiveIconFrame(look: look.1, colours: colours, strength: look.2))
                    guard !overlay.isHidden, let rep = overlay.bitmapImageRepForCachingDisplay(in: overlay.bounds) else { continue }
                    // The view's drawing alone; the breathing orb is laid over it below.
                    overlay.breathing.removeFromSuperlayer()
                    overlay.cacheDisplay(in: overlay.bounds, to: rep)
                    overlay.layer?.addSublayer(overlay.breathing)
                    rep.draw(in: NSRect(origin: origin, size: cell), from: .zero, operation: .sourceOver,
                             fraction: 1, respectFlipped: false, hints: nil)
                    // The breathing orb, at the breath's opacity: 1 breathed in, 0 breathed out.
                    if look.1.pulses, let cg = NSGraphicsContext.current?.cgContext {
                        cg.saveGState()
                        cg.translateBy(x: origin.x, y: origin.y)
                        cg.setAlpha(look.2)
                        overlay.breathing.render(in: cg)
                        cg.restoreGState()
                    }
                }
            }
            NSGraphicsContext.restoreGraphicsState()
            let png = try XCTUnwrap(strip.representation(using: .png, properties: [:]))
            try png.write(to: out.appendingPathComponent("menubar-\(name).png"))
        }
    }
}

@MainActor
final class StatusGlyphOverlayTests: XCTestCase {
    /// The overlay as drawn over a menu-bar button, 2x, `appearance`'s menu bar.
    private func snapshot(_ overlay: StatusGlyphOverlay, appearance: NSAppearance.Name = .aqua) throws -> NSBitmapImageRep {
        overlay.appearance = NSAppearance(named: appearance)
        let rep = try XCTUnwrap(overlay.bitmapImageRepForCachingDisplay(in: overlay.bounds))
        overlay.cacheDisplay(in: overlay.bounds, to: rep)
        return rep
    }

    private func colour(_ rep: NSBitmapImageRep, at point: CGPoint, in overlay: StatusGlyphOverlay) -> (GlowColours.RGB, CGFloat) {
        let scale = CGFloat(rep.pixelsWide) / overlay.bounds.width
        let glyph = StatusGlyphOverlay.glyphRect(in: overlay.bounds)
        let x = Int((glyph.minX + point.x) * scale)
        let y = rep.pixelsHigh - 1 - Int((glyph.minY + point.y) * scale)
        let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB)
        return (GlowColours.RGB(Double(c?.redComponent ?? 0), Double(c?.greenComponent ?? 0), Double(c?.blueComponent ?? 0)),
                c?.alphaComponent ?? 0)
    }

    func testTheOrbTakesTheStateColourOverTheTemplate() throws {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        overlay.show(LiveIconFrame(look: .rest, colours: colours, strength: 1))
        XCTAssertTrue(overlay.isHidden, "at rest only the template mark shows")

        overlay.show(LiveIconFrame(look: .glow(.you), colours: colours, strength: 1))
        XCTAssertFalse(overlay.isHidden)
        let (orb, alpha) = colour(try snapshot(overlay), at: CGPoint(x: 9, y: 9), in: overlay)
        XCTAssertEqual(alpha, 1, accuracy: 0.01)
        XCTAssertLessThan(distance(orb, colours.shown.you), 0.05, "the menu bar takes the mode shown's colours")
        let (rim, rimAlpha) = colour(try snapshot(overlay), at: CGPoint(x: 9, y: 15), in: overlay)
        XCTAssertEqual(rimAlpha, 0, accuracy: 0.01, "the rim stays the template's: \(rim)")
    }

    /// The alert colour is the menu bar's own appearance's, so it reads on either menu bar.
    func testTheAlertFollowsTheMenuBarsAppearance() throws {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        overlay.show(LiveIconFrame(look: .glow(.alert), colours: colours, strength: 1))
        let light = colour(try snapshot(overlay, appearance: .aqua), at: CGPoint(x: 9, y: 9), in: overlay).0
        let dark = colour(try snapshot(overlay, appearance: .darkAqua), at: CGPoint(x: 9, y: 9), in: overlay).0
        XCTAssertLessThan(distance(light, GlowColours.rgb(Glow.day.alert)), 0.05)
        XCTAssertLessThan(distance(dark, GlowColours.rgb(Glow.night.alert)), 0.05)
    }

    /// The final pass fills the rim clockwise from the top, in their colour.
    func testTheRingFillsTheRim() throws {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        overlay.show(LiveIconFrame(look: .ring(0.5), colours: colours, strength: 1))
        let rep = try snapshot(overlay)
        let (right, rightAlpha) = colour(rep, at: CGPoint(x: 15, y: 9), in: overlay)
        XCTAssertEqual(rightAlpha, 1, accuracy: 0.01)
        XCTAssertLessThan(distance(right, colours.shown.them), 0.05)
        XCTAssertEqual(colour(rep, at: CGPoint(x: 3, y: 9), in: overlay).1, 0, accuracy: 0.01, "still to come")
        XCTAssertEqual(colour(rep, at: CGPoint(x: 9, y: 9), in: overlay).1, 0, accuracy: 0.01, "the orb stays the template's")
    }

    /// The breath is a layer animation: run by the render server at the pulse's low rate, with
    /// no wakeup or redraw in the app. It starts with the pulse and stops with it.
    func testThePulseIsALayerAnimation() throws {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        XCTAssertTrue(overlay.breathesItself)
        overlay.show(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))
        let breath = try XCTUnwrap(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey) as? CABasicAnimation)
        XCTAssertEqual(breath.keyPath, "opacity")
        XCTAssertEqual(breath.fromValue as? Double, 1)
        XCTAssertEqual(breath.toValue as? Double, 0, "the colour fades into its tint")
        XCTAssertFalse(overlay.breathing.isHidden)
        let fill = try XCTUnwrap(overlay.breathing.fillColor.flatMap { NSColor(cgColor: $0)?.usingColorSpace(.sRGB) })
        XCTAssertLessThan(distance(GlowColours.RGB(fill.redComponent, fill.greenComponent, fill.blueComponent), colours.shown.them), 0.01)
        // Beneath it, the tint: the colour lighter, never darker, so never brown. (Seen with the
        // breathing orb taken off, as at the bottom of a breath; caching a view's display draws a
        // hidden sublayer all the same.)
        overlay.breathing.removeFromSuperlayer()
        let (tint, _) = colour(try snapshot(overlay), at: CGPoint(x: 9, y: 9), in: overlay)
        overlay.layer?.addSublayer(overlay.breathing)
        XCTAssertLessThan(distance(tint, GlowColours.partner(colours.shown.them)), 0.05)
        XCTAssertGreaterThan(GlowColours.luminance(tint), GlowColours.luminance(colours.shown.them))
        XCTAssertTrue(breath.autoreverses)
        XCTAssertEqual(breath.duration, LiveIcon.breathPeriod / 2)
        XCTAssertEqual(breath.repeatCount, .infinity)
        XCTAssertEqual(breath.preferredFrameRateRange.maximum, Float(LiveIcon.pulseFPS + 1))
        XCTAssertEqual(overlay.redraws, 1)

        overlay.show(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))
        XCTAssertEqual(overlay.redraws, 1, "the same pulse again: nothing")

        overlay.show(LiveIconFrame(look: .glow(.them), colours: colours, strength: 1))
        XCTAssertNil(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey), "stopped with the recording")
        XCTAssertTrue(overlay.breathing.isHidden)
        XCTAssertEqual(overlay.redraws, 2, "a new look is drawn")
        overlay.show(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))
        overlay.show(LiveIconFrame(look: .rest, colours: colours, strength: 1))
        XCTAssertNil(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey))
    }

    /// In a window: the breathing orb sits on the view's layer at the window's scale, over the
    /// orb's place, and the breath survives the view moving between windows.
    func testTheBreathHoldsInAWindow() throws {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        overlay.show(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))
        for _ in 0..<2 {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 24, height: 24), styleMask: .borderless, backing: .buffered,
                defer: false)
            window.isReleasedWhenClosed = false
            defer { window.close() }
            window.contentView?.addSubview(overlay)
            XCTAssertTrue(overlay.breathing.superlayer === overlay.layer)
            XCTAssertEqual(overlay.breathing.contentsScale, window.backingScaleFactor)
            XCTAssertNotNil(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey))
            let box = try XCTUnwrap(overlay.breathing.path?.boundingBox)
            let glyph = overlay.backingAlignedRect(StatusGlyphOverlay.glyphRect(in: overlay.bounds), options: .alignAllEdgesNearest)
            XCTAssertEqual(box, StatusGlyph.orb.offsetBy(dx: glyph.minX, dy: glyph.minY))
            overlay.removeFromSuperview()
        }
    }

    /// Asleep or locked, the breath is taken off the layer; awake, it is put back.
    func testTheBreathPausesWhileNobodyCanSee() {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        overlay.show(LiveIconFrame(look: .pulse(.them), colours: colours, strength: 1))
        overlay.setAwake(false)
        XCTAssertNil(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey))
        overlay.show(LiveIconFrame(look: .pulse(.you), colours: colours, strength: 1))
        XCTAssertNil(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey), "still asleep")
        overlay.setAwake(true)
        XCTAssertNotNil(overlay.breathing.animation(forKey: StatusGlyphOverlay.breathKey))
    }

    func testItTakesNoClicksAndSaysNothing() {
        let overlay = StatusGlyphOverlay(frame: NSRect(x: 0, y: 0, width: 24, height: 24))
        overlay.show(LiveIconFrame(look: .glow(.you), colours: colours, strength: 1))
        XCTAssertNil(overlay.hitTest(NSPoint(x: 12, y: 12)), "clicks reach the button and its menu")
        XCTAssertFalse(overlay.isAccessibilityElement(), "the button's label says the state")
    }

    func testTheSpokenLabelSaysTheState() {
        XCTAssertEqual(StatusItemController.spoken(.meeting), "Inkwell, recording")
        XCTAssertEqual(StatusItemController.spoken(.idle), "Inkwell")
    }
}
