// The shell's own logic: the routes the sidebar lists, where the data lives, the single-instance
// lock and the design tokens; the main window's frame and the height of its toolbar. (The rest of
// the window is checked by hand: mac/VOICEOVER-CHECKLIST.md.)
import AppKit
import Foundation
import InkBridge
import XCTest

@testable import Inkwell

final class RouterTests: XCTestCase {
    func testEveryRouteHasATitleAndASymbolThatExists() {
        for route in Route.allCases {
            XCTAssertFalse(route.title.isEmpty, "\(route)")
            XCTAssertNotNil(NSImage(systemSymbolName: route.symbol, accessibilityDescription: nil),
                            "\(route): no SF Symbol named \(route.symbol)")
        }
    }

    func testTheSidebarListsLiveOnlyWhileAMeetingIsLive() {
        let idle = SidebarSection.allCases.flatMap { $0.routes(meetingLive: false) }
        XCTAssertEqual(idle, [.today, .library, .owed, .settings])
        let live = SidebarSection.allCases.flatMap { $0.routes(meetingLive: true) }
        XCTAssertEqual(live, [.today, .library, .owed, .live, .settings])
        XCTAssertEqual(SidebarSection.recording.routes(meetingLive: false), [], "no empty section")
    }

    @MainActor
    func testAWindowShowingLiveGoesToTodayWhenTheMeetingEnds() {
        let router = Router()
        XCTAssertEqual(router.current, .today)
        router.open(.live)
        router.reconcile(meetingLive: true)
        XCTAssertEqual(router.current, .live)
        router.reconcile(meetingLive: false)
        XCTAssertEqual(router.selection, .today)

        router.open(.library)
        router.reconcile(meetingLive: false)
        XCTAssertEqual(router.selection, .library, "other screens stay where they are")

        router.selection = nil
        XCTAssertEqual(router.current, .today, "a cleared selection shows Today")
    }
}

/// The main window's frame, fitted to its screen at launch: a saved frame from a larger display
/// (or the default size on a small one) must never leave an edge out of reach.
final class WindowFrameTests: XCTestCase {
    /// A 1728 x 1117 pt display's visible frame (under the menu bar).
    private let screen = NSRect(x: 0, y: 0, width: 1728, height: 1079)
    private let minimum = NSSize(width: 720, height: 460)

    func testAFrameWiderThanTheScreenIsNarrowedToItAndBroughtOnScreen() {
        // As observed: 1755 pt wide on a 1728 pt display, its left edge at x = -51.
        let fitted = WindowFrame.fitted(NSRect(x: -51, y: 120, width: 1755, height: 760), in: screen, minSize: minimum)
        XCTAssertEqual(fitted, NSRect(x: 0, y: 120, width: 1728, height: 760))
    }

    func testAFrameOffTheLeftEdgeMovesBackOnScreenAtItsSize() {
        let fitted = WindowFrame.fitted(NSRect(x: -300, y: 120, width: 1040, height: 700), in: screen, minSize: minimum)
        XCTAssertEqual(fitted, NSRect(x: 0, y: 120, width: 1040, height: 700))
        let right = WindowFrame.fitted(NSRect(x: 1500, y: 120, width: 1040, height: 700), in: screen, minSize: minimum)
        XCTAssertEqual(right, NSRect(x: 688, y: 120, width: 1040, height: 700), "nor off the right edge")
    }

    func testAFrameTallerThanTheScreenIsShortenedToIt() {
        let fitted = WindowFrame.fitted(NSRect(x: 100, y: -50, width: 1040, height: 1300), in: screen, minSize: minimum)
        XCTAssertEqual(fitted, NSRect(x: 100, y: 0, width: 1040, height: 1079))
    }

    func testAFrameThatFitsIsLeftAlone() {
        let frame = NSRect(x: 200, y: 150, width: 1040, height: 700)
        XCTAssertEqual(WindowFrame.fitted(frame, in: screen, minSize: minimum), frame)
        // A second display to the right of the first: its own coordinates.
        let second = NSRect(x: 1728, y: -200, width: 1920, height: 1055)
        let there = NSRect(x: 2000, y: 0, width: 1040, height: 700)
        XCTAssertEqual(WindowFrame.fitted(there, in: second, minSize: minimum), there)
    }

    func testTheMinimumSizeWinsOnAScreenSmallerThanIt() {
        let tiny = NSRect(x: 0, y: 0, width: 700, height: 400)
        let fitted = WindowFrame.fitted(NSRect(x: -40, y: -40, width: 1040, height: 700), in: tiny, minSize: minimum)
        XCTAssertEqual(fitted.size, minimum)
        XCTAssertEqual(fitted.minX, 0, "the left edge stays reachable")
        XCTAssertEqual(fitted.maxY, 400, "and the title bar")
    }
}

/// The main window's chrome (title bar and toolbar) is as tall on every screen: a screen with no
/// toolbar item of its own once dropped the toolbar, and the title and window buttons moved up.
@MainActor
final class WindowChromeTests: XCTestCase {
    private struct NoCalendar: CalendarAccess {
        func state() -> CardState { .notAsked }
        func request(done: @escaping @MainActor @Sendable () -> Void) {}
    }

    func testTheToolbarIsAsTallOnEveryScreen() throws {
        _ = NSApplication.shared
        let store = CoreStore()
        let router = Router()
        let controller = MainWindowController(
            router: router, store: store, ink: ShellInk(store: store), updates: Updates(infoDictionary: nil),
            screens: ScreenModels(send: { _ in }, calendar: NoCalendar(), apps: WorkspaceApps()),
            library: LibraryModel(send: { _ in }))
        let window = try XCTUnwrap(controller.window)
        // Never shown: SwiftUI bridges the toolbar as it lays the window out.
        var heights: [Route: CGFloat] = [:]
        for route in Route.allCases {
            router.open(route)
            RunLoop.main.run(until: Date().addingTimeInterval(0.2))
            window.contentView?.layoutSubtreeIfNeeded()
            heights[route] = window.frame.height - window.contentLayoutRect.height
        }
        XCTAssertEqual(Set(heights.values).count, 1, "the chrome's height per screen: \(heights)")
        XCTAssertNotNil(window.toolbar)
    }
}

final class DataLocationTests: XCTestCase {
    func testTheDefaultIsApplicationSupportInkwell() throws {
        let data = try DataLocation.dataDirectory(environment: [:])
        XCTAssertEqual(data.lastPathComponent, "Inkwell")
        XCTAssertEqual(data.deletingLastPathComponent().lastPathComponent, "Application Support")
        XCTAssertNil(try DataLocation.modelsDirectory(environment: [:]))
        XCTAssertEqual(DataLocation.instanceLockFile().deletingLastPathComponent(), DataLocation.defaultDataDirectory())
    }

    func testOverridesMustBeAbsolute() throws {
        let env = ["INK_DATA_DIR": "/tmp/ink-data", "INK_MODELS_DIR": "/tmp/ink-models/"]
        XCTAssertEqual(try DataLocation.dataDirectory(environment: env).path, "/tmp/ink-data")
        XCTAssertEqual(try DataLocation.modelsDirectory(environment: env)?.path, "/tmp/ink-models")
        XCTAssertTrue(DataLocation.isMoved(environment: env))
        XCTAssertFalse(DataLocation.isMoved(environment: [:]))
        XCTAssertFalse(DataLocation.isMoved(environment: ["INK_DATA_DIR": ""]))
        XCTAssertThrowsError(try DataLocation.dataDirectory(environment: ["INK_DATA_DIR": "relative/dir"]))
        XCTAssertThrowsError(try DataLocation.modelsDirectory(environment: ["INK_MODELS_DIR": "~/models"]))
        XCTAssertEqual(try DataLocation.dataDirectory(environment: ["INK_DATA_DIR": ""]), DataLocation.defaultDataDirectory(),
                       "an empty override is no override")
    }

    func testTheLockDoesNotMoveWithTheLibrary() {
        // Two copies with different libraries would still share the dictation key and the mic.
        XCTAssertEqual(DataLocation.instanceLockFile().deletingLastPathComponent(), DataLocation.defaultDataDirectory())
    }
}

final class InstanceLockTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-lock-\(UUID().uuidString)")
        try DataLocation.create(directory)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    func testASecondCopyFindsTheLockHeldUntilTheFirstLetsGo() throws {
        let file = directory.appendingPathComponent("inkwell.lock")
        guard case .acquired(let first) = InstanceLock.acquire(at: file) else {
            return XCTFail("the first copy takes the lock")
        }
        // flock(2) locks belong to an open file, so a second open in this process stands in for a
        // second process.
        guard case .heldElsewhere = InstanceLock.acquire(at: file) else {
            return XCTFail("a second copy must find the lock held")
        }
        first.release()
        guard case .acquired(let second) = InstanceLock.acquire(at: file) else {
            return XCTFail("the lock is free once the first copy lets go (or dies)")
        }
        second.release()

        let mode = try FileManager.default.attributesOfItem(atPath: file.path)[.posixPermissions] as? Int
        XCTAssertEqual(mode, 0o600)
    }

    func testAnUnopenableLockFileIsAFailureNotAHeldLock() {
        let file = directory.appendingPathComponent("missing-dir/inkwell.lock")
        guard case .failed(let error) = InstanceLock.acquire(at: file) else {
            return XCTFail("a lock file that cannot be created is a failure")
        }
        XCTAssertEqual(error, ENOENT)
    }
}

final class DesignTokenTests: XCTestCase {
    /// The tokens' table, value for value.
    func testTheModesAreTheTokens() {
        XCTAssertEqual(Glow.day.background.hex, 0xFBF8F4)
        XCTAssertEqual(Glow.night.background.hex, 0x121118)
        XCTAssertEqual(Glow.day.text.hex, 0x1D1B2E)
        XCTAssertEqual(Glow.night.text.hex, 0xEDEAF2)
        XCTAssertEqual(Glow.day.alert.hex, 0xB23A26)
        XCTAssertEqual(Glow.night.alert.hex, 0xFF9A80)
        XCTAssertEqual(Glow.day.cardAlpha, 0.72)
        XCTAssertEqual(Glow.night.cardAlpha, 0.62)
        XCTAssertEqual(Glow.night.ink.hex, 0xF0EBE3)
        XCTAssertEqual(Glow.presets.map(\.id), ["indigo", "dusk", "lagoon", "aurora", "citrus", "rosewater", "ink_sand"])
        XCTAssertEqual(Glow.preset("nonsense").id, "indigo", "an unknown preset is the default")
    }

    /// The app's accent (Info.plist's NSAccentColorName, compiled from Assets.xcassets into the
    /// bundle by build-mac.sh) is the button fill of each mode, as Windows' SystemAccentColor is.
    func testTheAppAccentIsEachModesButtonFill() throws {
        let mac = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent()
        let plist = try XCTUnwrap(NSDictionary(contentsOf: mac.appendingPathComponent("Info.plist")))
        let name = try XCTUnwrap(plist["NSAccentColorName"] as? String)
        let colorset = mac.appendingPathComponent("Assets.xcassets/\(name).colorset/Contents.json")
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: colorset)) as? [String: Any])
        let colors = try XCTUnwrap(json["colors"] as? [[String: Any]])
        func hex(dark: Bool) throws -> UInt32 {
            let entry = try XCTUnwrap(colors.first { entry in
                let appearances = entry["appearances"] as? [[String: String]] ?? []
                return dark ? appearances == [["appearance": "luminosity", "value": "dark"]] : appearances.isEmpty
            }, dark ? "a dark entry" : "a universal entry")
            let color = try XCTUnwrap(entry["color"] as? [String: Any])
            XCTAssertEqual(color["color-space"] as? String, "srgb")
            let components = try XCTUnwrap(color["components"] as? [String: String])
            XCTAssertEqual(components["alpha"], "1.000")
            let channels = try ["red", "green", "blue"].map { key in
                let text = try XCTUnwrap(components[key]).dropFirst(2)
                return try XCTUnwrap(UInt32(text, radix: 16), key)
            }
            return channels[0] << 16 | channels[1] << 8 | channels[2]
        }
        XCTAssertEqual(try hex(dark: false), Glow.day.buttonFill.hex)
        XCTAssertEqual(try hex(dark: true), Glow.night.buttonFill.hex)
        XCTAssertEqual(colors.count, 2)
    }

    /// The words on a selected row, which AppKit fills with the accent: the button label on the
    /// button fill, and on a user's own accent whichever of the two labels reads.
    func testWordsOnTheAccentAreTheButtonLabel() {
        func label(_ swatch: Swatch) -> Swatch { Theme.label(onAccent: GlowColours.rgb(swatch)) }
        XCTAssertEqual(label(Glow.day.buttonFill), Glow.day.buttonLabel)
        XCTAssertEqual(label(Glow.night.buttonFill), Glow.night.buttonLabel)
        XCTAssertEqual(Theme.label(onAccent: .init(0, 0.48, 1)), Glow.day.buttonLabel, "the system's blue: light words")
        XCTAssertEqual(Theme.label(onAccent: .init(1, 0.78, 0)), Glow.night.buttonLabel, "its yellow: dark words")
    }

    func testASwatchIsItsSRGBValue() {
        let background = Glow.day.background.nsColor.usingColorSpace(.sRGB)
        XCTAssertEqual(background.map { Int(($0.redComponent * 255).rounded()) }, 0xFB)
        XCTAssertEqual(background.map { Int(($0.greenComponent * 255).rounded()) }, 0xF8)
        XCTAssertEqual(background.map { Int(($0.blueComponent * 255).rounded()) }, 0xF4)
    }

    func testTheSurfaceFollowsTheAppearance() {
        for (name, hex) in [(NSAppearance.Name.aqua, 0xFB), (.darkAqua, 0x12)] {
            var surface: NSColor?
            NSAppearance(named: name)?.performAsCurrentDrawingAppearance {
                surface = NSColor(Theme.surface).usingColorSpace(.sRGB)
            }
            XCTAssertEqual(surface.map { Int(($0.redComponent * 255).rounded()) }, hex, "\(name)")
        }
    }

    /// The resolve both shells share: a colour too dark for night is lifted, one too pale for day
    /// deepened, and the orb's partner is lighter.
    func testColoursResolveAsTheContractSays() throws {
        let navy = try XCTUnwrap(GlowColours.parse("#101030"))
        let lifted = GlowColours.fit(navy, dark: true)
        XCTAssertEqual(lifted.x, navy.x + (1 - navy.x) * 0.45, accuracy: 1e-9)
        XCTAssertEqual(GlowColours.fit(navy, dark: false), navy, "dark enough for day")
        let pale = try XCTUnwrap(GlowColours.parse("#fafaf0"))
        XCTAssertEqual(GlowColours.fit(pale, dark: false).x, pale.x * 0.7, accuracy: 1e-9)
        XCTAssertEqual(GlowColours.fit(pale, dark: true), pale)
        XCTAssertEqual(GlowColours.partner(navy).y, navy.y + (1 - navy.y) * 0.4, accuracy: 1e-9)

        let indigo = Glow.preset("indigo")
        let own = GlowColours.dots(preset: indigo, you: "#336699", them: nil, dark: false)
        XCTAssertEqual(GlowColours.hex(own.you), "#336699", "the user's own colour replaces the preset's")
        XCTAssertEqual(GlowColours.hex(own.them), "#ffa34d")
        XCTAssertNil(GlowColours.parse("#FFA34D"), "settings hold lowercase only")
        XCTAssertNil(GlowColours.parse("preset"))
    }
}

final class MeasurementTests: XCTestCase {
    @MainActor
    func testAStartedMeasurementIsReleasedWithItsOwner() throws {
        // The app's Measurement, not Foundation's.
        weak var released: Inkwell.Measurement?
        do {
            let measurement = try XCTUnwrap(Inkwell.Measurement.fromEnvironment(["INK_MEASURE": "idle"]))
            measurement.start { false }
            released = measurement
        }
        // Its signal source's handler must not keep it alive (a retain cycle through the source).
        XCTAssertNil(released)
    }

    func testNoMeasurementWithoutINK_MEASURE() {
        MainActor.assumeIsolated {
            XCTAssertNil(Inkwell.Measurement.fromEnvironment([:]))
            XCTAssertNil(Inkwell.Measurement.fromEnvironment(["INK_MEASURE": ""]))
        }
    }
}
