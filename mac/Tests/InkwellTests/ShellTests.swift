// The shell's own logic: the routes the sidebar lists, where the data lives, the single-instance
// lock and the design tokens. (The window itself is checked by hand: mac/VOICEOVER-CHECKLIST.md.)
import AppKit
import Foundation
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
    /// The canvas's table, value for value.
    func testThePaletteIsTheCanvas() {
        XCTAssertEqual(Palette.paper.hex, 0xF2EEE6)
        XCTAssertEqual(Palette.ink.hex, 0x16181F)
        XCTAssertEqual(Palette.sepia.hex, 0x7E5431)
        XCTAssertEqual(Palette.seal.hex, 0xB23A26)
        XCTAssertEqual(Palette.nightPaper.hex, 0x121419)
        XCTAssertEqual(Palette.muted.hex, 0x625E57)
    }

    func testASwatchIsItsSRGBValue() {
        let paper = Palette.paper.nsColor.usingColorSpace(.sRGB)
        XCTAssertEqual(paper.map { Int(($0.redComponent * 255).rounded()) }, 0xF2)
        XCTAssertEqual(paper.map { Int(($0.greenComponent * 255).rounded()) }, 0xEE)
        XCTAssertEqual(paper.map { Int(($0.blueComponent * 255).rounded()) }, 0xE6)
    }

    func testTheRailIsPaperInBothThemes() {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            var rail: NSColor?
            NSAppearance(named: name)?.performAsCurrentDrawingAppearance {
                rail = NSColor(Theme.inkZone).usingColorSpace(.sRGB)
            }
            XCTAssertEqual(rail.map { Int(($0.redComponent * 255).rounded()) }, 0xF2, "\(name)")
        }
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
