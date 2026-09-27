// A second copy of Inkwell asks the running one to show its window over a Unix-domain socket, and
// only a process of the same user, signed as this same app, is heard.
//
// Under `swift test` "this app" is the test runner: the server checks peers against the running
// process's own designated requirement, as the app does, so an in-process client passes and any
// other program fails.
import Darwin
import Foundation
import Security
import Synchronization
import XCTest

@testable import Inkwell

/// A short directory for sockets (sun_path holds 104 bytes), removed afterwards.
private func socketDirectory() throws -> URL {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("ink-\(UUID().uuidString.prefix(8))", isDirectory: true)
    try DataLocation.create(dir)
    return dir
}

/// Counts show requests, from any thread.
private final class Shown: Sendable {
    private let count = Mutex(0)
    func record() { count.withLock { $0 += 1 } }
    var total: Int { count.withLock { $0 } }
}

final class ShowWindowChannelTests: XCTestCase {
    private var dir: URL!
    private var path: String { dir.appendingPathComponent("inkwell.sock").path }

    override func setUpWithError() throws {
        dir = try socketDirectory()
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    func testTheSameAppAsTheSameUserIsShown() throws {
        let shown = Shown()
        let channel = try ShowWindowChannel.listen(at: path, onShow: { shown.record() })
        defer { channel.close() }
        XCTAssertEqual(ShowWindowChannel.requestShow(at: path, timeout: 2), .shown)
        XCTAssertEqual(shown.total, 1)

        let mode = try FileManager.default.attributesOfItem(atPath: path)[.posixPermissions] as? Int
        XCTAssertEqual(mode, 0o600, "the socket is this user's only")
    }

    func testAnotherUsersClientIsRefused() throws {
        let shown = Shown()
        // getpeereid reports this process's UID, which is not the one the server expects.
        let channel = try ShowWindowChannel.listen(at: path, peerUID: getuid() &+ 1, onShow: { shown.record() })
        defer { channel.close() }
        XCTAssertEqual(ShowWindowChannel.requestShow(at: path, timeout: 2), .refused)
        XCTAssertEqual(shown.total, 0)
    }

    func testAClientSignedAsAnotherAppIsRefused() throws {
        let shown = Shown()
        var inkwell: SecRequirement?
        XCTAssertEqual(SecRequirementCreateWithString(
            #"identifier "com.inkwell.app" and anchor apple generic"# as CFString, [], &inkwell), errSecSuccess)
        let channel = try ShowWindowChannel.listen(at: path, requirement: inkwell, onShow: { shown.record() })
        defer { channel.close() }
        // This process (the test runner) is signed, but not as the app the server accepts.
        XCTAssertEqual(ShowWindowChannel.requestShow(at: path, timeout: 2), .refused)
        XCTAssertEqual(shown.total, 0)
    }

    func testAnotherProgramOfTheSameUserIsRefused() throws {
        let shown = Shown()
        let channel = try ShowWindowChannel.listen(at: path, onShow: { shown.record() })
        defer { channel.close() }

        // nc: the same user, a valid Apple signature, and not this app.
        let nc = Process()
        nc.executableURL = URL(fileURLWithPath: "/usr/bin/nc")
        nc.arguments = ["-U", path, "-w", "2"]
        let input = Pipe()
        let output = Pipe()
        nc.standardInput = input
        nc.standardOutput = output
        nc.standardError = FileHandle.nullDevice
        try nc.run()
        input.fileHandleForWriting.write(Data("show\n".utf8))
        try input.fileHandleForWriting.close()
        nc.waitUntilExit()
        let reply = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        XCTAssertEqual(reply, "", "no answer for a program that is not this app")
        XCTAssertEqual(shown.total, 0)
    }

    func testAnUnknownCommandIsIgnored() throws {
        let shown = Shown()
        let channel = try ShowWindowChannel.listen(at: path, onShow: { shown.record() })
        defer { channel.close() }
        XCTAssertEqual(ShowWindowChannel.request("hide", at: path, timeout: 2), .refused)
        XCTAssertEqual(shown.total, 0)
    }

    func testAStaleSocketFromACrashIsReplaced() throws {
        // What a crashed copy leaves: a socket file with no listener behind it.
        let stale = socket(AF_UNIX, SOCK_STREAM, 0)
        XCTAssertGreaterThanOrEqual(stale, 0)
        var address = try ShowWindowChannel.address(path)
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(stale, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        XCTAssertEqual(bound, 0)
        Darwin.close(stale)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path))

        // Nobody listens: a second copy gives up at its deadline instead of hanging.
        let start = Date()
        XCTAssertEqual(ShowWindowChannel.requestShow(at: path, timeout: 0.3), .unreachable)
        XCTAssertLessThan(Date().timeIntervalSince(start), 2)

        // The next copy to hold the lock takes the path over.
        let shown = Shown()
        let channel = try ShowWindowChannel.listen(at: path, onShow: { shown.record() })
        defer { channel.close() }
        XCTAssertEqual(ShowWindowChannel.requestShow(at: path, timeout: 2), .shown)
        XCTAssertEqual(shown.total, 1)
    }

    func testAClientThatArrivesBeforeTheListenerStillGetsThrough() throws {
        // A second copy launched while the first is still starting: it retries until the socket is
        // up.
        let outcome = Mutex<ShowWindowChannel.Outcome?>(nil)
        let done = expectation(description: "client finished")
        let socketPath = path
        DispatchQueue.global().async {
            let result = ShowWindowChannel.requestShow(at: socketPath, timeout: 5)
            outcome.withLock { $0 = result }
            done.fulfill()
        }
        Thread.sleep(forTimeInterval: 0.3)
        let shown = Shown()
        let channel = try ShowWindowChannel.listen(at: path, onShow: { shown.record() })
        defer { channel.close() }
        wait(for: [done], timeout: 10)
        XCTAssertEqual(outcome.withLock { $0 }, .shown)
        XCTAssertEqual(shown.total, 1)
    }

    func testAPathTooLongForASocketIsAnErrorNotATruncation() {
        let long = "/" + String(repeating: "x", count: 200) + "/inkwell.sock"
        XCTAssertThrowsError(try ShowWindowChannel.listen(at: long, onShow: {}))
        XCTAssertEqual(ShowWindowChannel.requestShow(at: long, timeout: 0.1), .unreachable)
    }
}

final class ShowRequestsTests: XCTestCase {
    @MainActor
    func testARequestBeforeTheAppIsReadyIsShownWhenItIs() {
        let requests = ShowRequests()
        var shows = 0
        requests.arrived()
        requests.arrived()
        XCTAssertEqual(shows, 0)
        requests.attach { shows += 1 }
        XCTAssertEqual(shows, 1, "requests made during startup show the window once, when it can")
        requests.arrived()
        XCTAssertEqual(shows, 2, "later requests show it at once")
    }

    @MainActor
    func testNoRequestNoShow() {
        let requests = ShowRequests()
        var shows = 0
        requests.attach { shows += 1 }
        XCTAssertEqual(shows, 0)
    }

    /// The path the app wires: socket → the channel's queue → the main actor → pending until the
    /// delegate attaches.
    @MainActor
    func testASecondLaunchDuringStartupShowsTheWindow() throws {
        let dir = try socketDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let path = dir.appendingPathComponent("inkwell.sock").path
        let requests = ShowRequests()
        let channel = try ShowWindowChannel.listen(at: path, onShow: ShowRequests.forwarder(to: requests))
        defer { channel.close() }

        // The app is still starting (nothing attached) when the second copy asks.
        let answered = expectation(description: "second copy answered")
        DispatchQueue.global().async {
            XCTAssertEqual(ShowWindowChannel.requestShow(at: path, timeout: 2), .shown)
            answered.fulfill()
        }
        wait(for: [answered], timeout: 5)
        let until = Date().addingTimeInterval(2)
        while !requests.waiting && Date() < until {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
        XCTAssertTrue(requests.waiting)

        var shows = 0
        requests.attach { shows += 1 }
        XCTAssertEqual(shows, 1)
    }
}
