// Inkwell 0.2's Open at Login carried over to this app's login item, once, on a fake file system
// and a fake login item: no test reads the real LaunchAgents folder or registers anything.
import Foundation
import XCTest

@testable import Inkwell

/// Files by URL. Records every read and removal.
private final class FakeFiles: LaunchAgentFiles {
    var contents: [URL: Data] = [:]
    var readError: (any Error)?
    var removeError: (any Error)?
    private(set) var reads: [URL] = []
    private(set) var removals: [URL] = []

    func read(_ file: URL) throws -> Data? {
        reads.append(file)
        if let readError { throw readError }
        return contents[file]
    }

    func remove(_ file: URL) throws {
        if let removeError { throw removeError }
        removals.append(file)
        contents[file] = nil
    }
}

/// A login item: `turnOn` moves it to `afterTurnOn`, then throws `turnOnError` if set.
@MainActor
private final class FakeLoginItem: LoginItemRegistrar {
    var state: LoginItem.State
    var afterTurnOn: LoginItem.State = .on
    var turnOnError: (any Error)?
    private(set) var turnOns = 0

    init(_ state: LoginItem.State = .off) {
        self.state = state
    }

    func turnOn() throws {
        turnOns += 1
        state = afterTurnOn
        if let turnOnError { throw turnOnError }
    }
}

/// 0.2's agent as 0.2 wrote it: the launch-agent file of Tauri's autostart plugin (auto-launch 0.5).
private func agent02(program: String = "/Applications/Inkwell.app/Contents/MacOS/app",
                     label: String = "Inkwell", runAtLoad: String = "<true/>") -> Data {
    Data("""
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
      <dict>
      <key>Label</key>
      <string>\(label)</string>
      <key>ProgramArguments</key>
      <array><string>\(program)</string></array>
      <key>RunAtLoad</key>
      \(runAtLoad)
      </dict>
    </plist>
    """.utf8)
}

private func plist(_ object: [String: Any]) throws -> Data {
    try PropertyListSerialization.data(fromPropertyList: object, format: .xml, options: 0)
}

@MainActor
final class LoginItemMigrationTests: XCTestCase {
    private let agent = URL(fileURLWithPath: "/nonexistent-home/Library/LaunchAgents/Inkwell.plist")
    private let bundle = URL(fileURLWithPath: "/Applications/Inkwell.app")
    private var files: FakeFiles!
    private var loginItem: FakeLoginItem!
    private var suite: String!
    private var defaults: UserDefaults!

    override func setUp() async throws {
        files = FakeFiles()
        loginItem = FakeLoginItem()
        suite = "ink-login-migration-\(UUID().uuidString)"
        defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: suite)
    }

    private func migration(bundle: URL? = nil) -> LoginItemMigration {
        LoginItemMigration(agent: agent, bundle: bundle ?? self.bundle, files: files, loginItem: loginItem,
                           defaults: defaults)
    }

    private var recorded: Bool { defaults.bool(forKey: LoginItemMigration.recordKey) }

    // MARK: - 0.2's agent for this bundle

    func testZeroTwosAgentTurnsTheLoginItemOnAndIsRemoved() {
        files.contents[agent] = agent02()
        let outcome = migration().run()
        XCTAssertEqual(outcome, .carriedOver(turnedOn: .success(.on), notRemoved: nil))
        XCTAssertEqual(loginItem.turnOns, 1)
        XCTAssertEqual(files.removals, [agent], "that one file, and nothing else")
        XCTAssertEqual(files.reads, [agent])
        XCTAssertTrue(recorded)
        XCTAssertNil(LoginItemMigration.notice(for: outcome), "nothing to tell")
    }

    func testZeroTwosAgentMatchesThisBundleReachedThroughALink() throws {
        // 0.2 wrote its executable's resolved path; this copy was opened through a link to it.
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ink-bundle-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let real = dir.appendingPathComponent("Real/Inkwell.app", isDirectory: true)
        try FileManager.default.createDirectory(at: real, withIntermediateDirectories: true)
        let link = dir.appendingPathComponent("Inkwell.app", isDirectory: true)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: real)
        let resolved = try XCTUnwrap(realpath(real.path, nil))
        defer { free(resolved) }
        let program = String(cString: resolved) + "/Contents/MacOS/app"
        XCTAssertNotEqual(link.path + "/Contents/MacOS/app", program, "the paths differ as given")

        files.contents[agent] = agent02(program: program)
        XCTAssertEqual(migration(bundle: link).run(), .carriedOver(turnedOn: .success(.on), notRemoved: nil))
        XCTAssertEqual(files.removals, [agent])
    }

    func testALoginItemWaitingForApprovalCountsAsOn() {
        files.contents[agent] = agent02()
        loginItem.afterTurnOn = .needsApproval
        XCTAssertEqual(migration().run(), .carriedOver(turnedOn: .success(.needsApproval), notRemoved: nil))
        XCTAssertEqual(files.removals, [agent])
    }

    func testAnErrorThatLeavesItWaitingForApprovalCountsAsOn() {
        // macOS can refuse the registration and still hold it for the user's approval.
        files.contents[agent] = agent02()
        loginItem.afterTurnOn = .needsApproval
        loginItem.turnOnError = NSError(domain: "SMAppServiceErrorDomain", code: 1)
        XCTAssertEqual(migration().run(), .carriedOver(turnedOn: .success(.needsApproval), notRemoved: nil))
    }

    func testAFailedTurnOnIsShownAndTheDeadAgentStillGoes() throws {
        files.contents[agent] = agent02()
        loginItem.afterTurnOn = .off
        loginItem.turnOnError = NSError(domain: "SMAppServiceErrorDomain", code: 1,
                                        userInfo: [NSLocalizedDescriptionKey: "Operation not permitted."])
        let outcome = migration().run()
        XCTAssertEqual(outcome, .carriedOver(
            turnedOn: .failure(.init(domain: "SMAppServiceErrorDomain", code: 1, description: "Operation not permitted.")),
            notRemoved: nil))
        XCTAssertEqual(files.removals, [agent], "it opens an executable this bundle does not have")
        XCTAssertTrue(recorded)
        let notice = try XCTUnwrap(LoginItemMigration.notice(for: outcome))
        XCTAssertEqual(notice.title, "Inkwell could not turn on Open at Login.")
        XCTAssertTrue(notice.detail.contains("Operation not permitted."), notice.detail)
        XCTAssertTrue(notice.detail.contains("menu-bar item"), notice.detail)
    }

    func testATurnOnThatLeavesItOffWithoutAnErrorIsAFailureToo() {
        files.contents[agent] = agent02()
        loginItem.afterTurnOn = .off
        guard case .carriedOver(.failure, nil) = migration().run() else {
            return XCTFail("a login item still off is not carried over")
        }
    }

    func testAFailedRemovalIsShownNamingTheFile() throws {
        files.contents[agent] = agent02()
        files.removeError = NSError(domain: NSCocoaErrorDomain, code: NSFileWriteNoPermissionError,
                                    userInfo: [NSLocalizedDescriptionKey: "No permission."])
        let outcome = migration().run()
        XCTAssertEqual(outcome, .carriedOver(
            turnedOn: .success(.on),
            notRemoved: .init(domain: NSCocoaErrorDomain, code: NSFileWriteNoPermissionError, description: "No permission.")))
        XCTAssertTrue(recorded, "said once, not at every launch")
        let notice = try XCTUnwrap(LoginItemMigration.notice(for: outcome))
        XCTAssertEqual(notice.title, "Inkwell could not remove Inkwell 0.2\u{2019}s login item.")
        XCTAssertTrue(notice.detail.hasPrefix("Open at Login is on."), notice.detail)
        XCTAssertTrue(notice.detail.contains("~/Library/LaunchAgents/Inkwell.plist"), notice.detail)
        XCTAssertTrue(notice.detail.contains("No permission."), notice.detail)
    }

    func testBothFailuresAreShownTogether() throws {
        files.contents[agent] = agent02()
        loginItem.afterTurnOn = .off
        loginItem.turnOnError = NSError(domain: "SMAppServiceErrorDomain", code: 1)
        files.removeError = NSError(domain: NSCocoaErrorDomain, code: NSFileWriteNoPermissionError)
        let notice = try XCTUnwrap(LoginItemMigration.notice(for: migration().run()))
        XCTAssertEqual(notice.title, "Inkwell could not turn on Open at Login.")
        XCTAssertTrue(notice.detail.contains("~/Library/LaunchAgents/Inkwell.plist"), notice.detail)
    }

    func testARemovalFailureWhileWaitingForApprovalSaysSo() throws {
        let outcome = LoginItemMigration.Outcome.carriedOver(
            turnedOn: .success(.needsApproval), notRemoved: .init(domain: "d", code: 1, description: "Busy."))
        let notice = try XCTUnwrap(LoginItemMigration.notice(for: outcome))
        XCTAssertTrue(notice.detail.hasPrefix("Open at Login is waiting for your approval in System Settings"),
                      notice.detail)
    }

    // MARK: - Once

    func testItRunsOnce() {
        files.contents[agent] = agent02()
        _ = migration().run()
        // 0.2's agent back (0.2 reinstalled and switched on, say) and the login item off: the
        // user's later choice stands.
        files.contents[agent] = agent02()
        loginItem.state = .off
        XCTAssertEqual(migration().run(), .alreadyRecorded)
        XCTAssertEqual(files.reads.count, 1, "nothing read the second time")
        XCTAssertEqual(loginItem.turnOns, 1)
        XCTAssertEqual(files.removals.count, 1)
    }

    func testNoAgentIsSettledWithoutTouchingTheLoginItem() {
        XCTAssertEqual(migration().run(), .no02Agent)
        XCTAssertEqual(loginItem.turnOns, 0)
        XCTAssertTrue(recorded)
    }

    // MARK: - Left alone

    func testAFileByThatNameThatIsNotZeroTwosIsLeftAlone() throws {
        let others: [(String, Data)] = [
            ("another label", agent02(label: "com.example.inkwell")),
            ("another program", agent02(program: "/Applications/Inkwell.app/Contents/MacOS/Inkwell")),
            ("a relative program", agent02(program: "Inkwell.app/Contents/MacOS/app")),
            ("not run at load", agent02(runAtLoad: "<false/>")),
            ("no run at load", try plist(["Label": "Inkwell",
                                           "ProgramArguments": ["/Applications/Inkwell.app/Contents/MacOS/app"]])),
            ("two arguments", try plist(["Label": "Inkwell", "RunAtLoad": true,
                                         "ProgramArguments": ["/Applications/Inkwell.app/Contents/MacOS/app", "--x"]])),
            ("a program key", try plist(["Label": "Inkwell", "RunAtLoad": true,
                                         "Program": "/Applications/Inkwell.app/Contents/MacOS/app"])),
            ("not a plist", Data("Label=Inkwell".utf8)),
            ("empty", Data()),
        ]
        for (name, data) in others {
            defaults.removeObject(forKey: LoginItemMigration.recordKey)
            files.contents[agent] = data
            XCTAssertEqual(migration().run(), .notInkwell02, name)
            XCTAssertTrue(recorded, name)
        }
        XCTAssertEqual(loginItem.turnOns, 0)
        XCTAssertEqual(files.removals, [])
    }

    func testZeroTwosAgentForAnotherCopyIsLeftAloneAndAskedAgainLater() {
        // A development build, or 1.0 beside 0.2: that agent still opens 0.2.
        files.contents[agent] = agent02(program: "/Applications/Inkwell.app/Contents/MacOS/app")
        for elsewhere in ["/Applications/Inkwell Beta.app", "/Volumes/Spare/Inkwell.app", "/Applications/Inkwell"] {
            XCTAssertEqual(migration(bundle: URL(fileURLWithPath: elsewhere)).run(), .anotherCopy, elsewhere)
        }
        XCTAssertEqual(loginItem.turnOns, 0)
        XCTAssertEqual(files.removals, [])
        XCTAssertFalse(recorded, "the copy it names may still launch")
        // The copy it names, later: carried over.
        XCTAssertEqual(migration().run(), .carriedOver(turnedOn: .success(.on), notRemoved: nil))
    }

    func testNotABundleReadsNothingAndRecordsNothing() {
        loginItem.state = .unavailable
        files.contents[agent] = agent02()
        XCTAssertEqual(migration().run(), .notABundle)
        XCTAssertEqual(files.reads, [])
        XCTAssertFalse(recorded)
    }

    func testAnUnreadableFileIsLeftAloneAndAskedAgainLater() {
        files.readError = POSIXError(.EACCES)
        guard case .unreadable(let failure) = migration().run() else { return XCTFail("unreadable") }
        XCTAssertEqual(failure.domain, NSPOSIXErrorDomain)
        XCTAssertEqual(failure.code, Int(EACCES))
        XCTAssertEqual(loginItem.turnOns, 0)
        XCTAssertFalse(recorded)
    }

    func testOnlyFailuresAreShown() {
        for outcome: LoginItemMigration.Outcome in [
            .alreadyRecorded, .notABundle, .no02Agent, .notInkwell02, .anotherCopy,
            .unreadable(.init(domain: "d", code: 1, description: "x")),
            .carriedOver(turnedOn: .success(.on), notRemoved: nil),
            .carriedOver(turnedOn: .success(.needsApproval), notRemoved: nil),
        ] {
            XCTAssertNil(LoginItemMigration.notice(for: outcome), "\(outcome)")
        }
    }
}

/// The real file system, in a temporary directory.
final class SystemLaunchAgentFilesTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ink-agents-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    func testAMissingFileReadsAsNothing() throws {
        XCTAssertNil(try SystemLaunchAgentFiles().read(dir.appendingPathComponent("Inkwell.plist")))
    }

    func testARegularFileIsReadAndRemoved() throws {
        let file = dir.appendingPathComponent("Inkwell.plist")
        try agent02().write(to: file)
        XCTAssertEqual(try SystemLaunchAgentFiles().read(file), agent02())
        try SystemLaunchAgentFiles().remove(file)
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
    }

    func testALinkOrAFolderIsNotRead() throws {
        let target = dir.appendingPathComponent("elsewhere.plist")
        try agent02().write(to: target)
        let link = dir.appendingPathComponent("Inkwell.plist")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)
        XCTAssertThrowsError(try SystemLaunchAgentFiles().read(link))

        let folder = dir.appendingPathComponent("Folder.plist", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        XCTAssertThrowsError(try SystemLaunchAgentFiles().read(folder))
    }

    @MainActor
    func testTheBundlePathIsComparedAfterResolvingLinks() throws {
        // 0.2 wrote its executable's resolved path; a bundle reached through a link still matches.
        let real = dir.appendingPathComponent("Real/Inkwell.app", isDirectory: true)
        try FileManager.default.createDirectory(at: real, withIntermediateDirectories: true)
        let link = dir.appendingPathComponent("Linked")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: real.deletingLastPathComponent())
        let viaLink = link.appendingPathComponent("Inkwell.app", isDirectory: true)
        XCTAssertEqual(LoginItemMigration.canonicalPath(viaLink), LoginItemMigration.canonicalPath(real))
        XCTAssertFalse(LoginItemMigration.canonicalPath(real).contains("/Linked/"))
        // A path that does not exist is compared as given.
        XCTAssertEqual(LoginItemMigration.canonicalPath(URL(fileURLWithPath: "/nonexistent/Inkwell.app")),
                       "/nonexistent/Inkwell.app")
    }
}
