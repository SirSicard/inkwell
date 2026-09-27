// How a second copy of Inkwell asks the running one to show its window: a Unix-domain socket in
// the data directory (0700), created by whichever copy holds the single-instance lock, before the
// core starts.
//
// A request is heard only from a peer that is
// - the same user (getpeereid), and
// - signed as this same app: its audit token's code satisfies the running app's own designated
//   requirement (SecCode). A Developer ID build accepts any build with the same identifier and
//   team; an ad-hoc build accepts only itself.
// Anything else is closed unanswered. A distributed notification cannot do this: it carries no
// sender, so any local process could post it.
//
// The protocol is one line each way: the client sends "show\n"; the server answers "ok\n" when
// it accepted the request, and closes. Stale sockets (a crashed copy's) need no detection: only
// the lock holder ever binds, and it replaces whatever is at the path.
import Darwin
import Foundation
import Security
import os

final class ShowWindowChannel: Sendable {
    /// What a client's request came to.
    enum Outcome: Equatable, Sendable {
        /// The running copy accepted it.
        case shown
        /// Something listens, and it did not accept the request (another user or app, or a
        /// command it does not know).
        case refused
        /// Nothing listened before the deadline.
        case unreachable
    }

    /// A socket call failed.
    struct Failure: Error, CustomStringConvertible {
        let call: String
        let code: Int32
        var description: String { "\(call): \(String(cString: strerror(code)))" }
    }

    static let showCommand = "show"
    private static let accepted = "ok\n"
    private static let log = Logger(subsystem: "com.inkwell.app", category: "instance")

    private let source: any DispatchSourceRead

    private init(source: any DispatchSourceRead) {
        self.source = source
    }

    // MARK: Server

    /// Listens at `path`, replacing whatever is there: call it only while holding the
    /// single-instance lock. `onShow` runs on the channel's own queue for every accepted request.
    ///
    /// `requirement` defaults to this process's own designated requirement, and `peerUID` to this
    /// process's user (tests pass others).
    static func listen(
        at path: String,
        requirement: SecRequirement? = nil,
        peerUID: uid_t = getuid(),
        onShow: @escaping @Sendable () -> Void
    ) throws -> ShowWindowChannel {
        var address = try address(path)
        let required = try requirementData(requirement ?? ownRequirement())

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw Failure(call: "socket", code: errno) }
        var ready = false
        defer {
            if !ready { Darwin.close(fd) }
        }
        _ = fcntl(fd, F_SETFD, FD_CLOEXEC)
        // A crashed copy's socket, or anything else left at the path: the lock says it is ours.
        if unlink(path) != 0 && errno != ENOENT {
            throw Failure(call: "unlink", code: errno)
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else { throw Failure(call: "bind", code: errno) }
        guard chmod(path, 0o600) == 0 else { throw Failure(call: "chmod", code: errno) }
        guard Darwin.listen(fd, 8) == 0 else { throw Failure(call: "listen", code: errno) }
        // Non-blocking, so the accept loop below stops when the backlog is empty.
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)

        let queue = DispatchQueue(label: "com.inkwell.app.show-window")
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler {
            while true {
                let connection = accept(fd, nil, nil)
                guard connection >= 0 else { return }
                serve(connection, required: required, peerUID: peerUID, onShow: onShow)
            }
        }
        source.setCancelHandler {
            Darwin.close(fd)
        }
        source.resume()
        ready = true
        return ShowWindowChannel(source: source)
    }

    /// Stops listening. The socket file stays; the next lock holder replaces it.
    func close() {
        source.cancel()
    }

    private static func serve(
        _ connection: Int32, required: Data, peerUID: uid_t, onShow: @Sendable () -> Void
    ) {
        defer { Darwin.close(connection) }
        // accept() hands back the listener's O_NONBLOCK on this platform: the reads below wait,
        // bounded by the timeouts.
        _ = fcntl(connection, F_SETFL, fcntl(connection, F_GETFL) & ~O_NONBLOCK)
        configure(connection, timeout: 1)

        switch authenticate(connection, required: required, peerUID: peerUID) {
        case .allowed:
            break
        case .refused(let reason):
            log.notice("refused a show-window request: \(reason, privacy: .public)")
            return
        }
        // Read only once the peer is known to be this app, so no other process can hold the
        // queue with a slow write.
        guard readLine(connection, limit: 16) == showCommand else { return }
        // Handed on first, then answered: "shown" tells the second copy the request reached this
        // one, not only that it was read.
        onShow()
        _ = accepted.withCString { write(connection, $0, strlen($0)) }
    }

    private enum Verdict {
        case allowed
        case refused(String)
    }

    private static func authenticate(_ connection: Int32, required: Data, peerUID: uid_t) -> Verdict {
        var uid = uid_t()
        var gid = gid_t()
        guard getpeereid(connection, &uid, &gid) == 0 else {
            return .refused("no peer credentials")
        }
        guard uid == peerUID else {
            return .refused("another user (uid \(uid))")
        }
        // The audit token, not the pid: a pid can be reused by another process before it is
        // checked, a token names exactly the process on the other end.
        var token = audit_token_t()
        var length = socklen_t(MemoryLayout<audit_token_t>.size)
        guard getsockopt(connection, SOL_LOCAL, LOCAL_PEERTOKEN, &token, &length) == 0 else {
            return .refused("no audit token")
        }
        let tokenData = withUnsafeBytes(of: &token) { Data($0) }
        var code: SecCode?
        let attributes = [kSecGuestAttributeAudit: tokenData] as CFDictionary
        guard SecCodeCopyGuestWithAttributes(nil, attributes, [], &code) == errSecSuccess, let code else {
            return .refused("the peer's code could not be read")
        }
        var requirement: SecRequirement?
        guard SecRequirementCreateWithData(required as CFData, [], &requirement) == errSecSuccess,
            let requirement
        else {
            return .refused("the requirement could not be read")
        }
        let status = SecCodeCheckValidity(code, [], requirement)
        guard status == errSecSuccess else {
            return .refused("not signed as this app (status \(status))")
        }
        return .allowed
    }

    /// This process's designated requirement.
    static func ownRequirement() throws -> SecRequirement {
        var me: SecCode?
        var file: SecStaticCode?
        var requirement: SecRequirement?
        guard SecCodeCopySelf([], &me) == errSecSuccess, let me,
            SecCodeCopyStaticCode(me, [], &file) == errSecSuccess, let file,
            SecCodeCopyDesignatedRequirement(file, [], &requirement) == errSecSuccess, let requirement
        else {
            throw Failure(call: "SecCodeCopyDesignatedRequirement", code: 0)
        }
        return requirement
    }

    /// The requirement in its binary form: plain data, so the listener's queue can hold it (the
    /// SecRequirement object is not Sendable).
    private static func requirementData(_ requirement: SecRequirement) throws -> Data {
        var data: CFData?
        guard SecRequirementCopyData(requirement, [], &data) == errSecSuccess, let data else {
            throw Failure(call: "SecRequirementCopyData", code: 0)
        }
        return data as Data
    }

    // MARK: Client

    /// Asks the copy listening at `path` to show its window. Retries while nothing listens yet
    /// (that copy may still be starting) until `timeout` has passed.
    static func requestShow(at path: String, timeout: TimeInterval) -> Outcome {
        request(showCommand, at: path, timeout: timeout)
    }

    /// Sends one command line and reads the answer.
    static func request(_ command: String, at path: String, timeout: TimeInterval) -> Outcome {
        guard var address = try? address(path) else { return .unreachable }
        let deadline = Date().addingTimeInterval(timeout)
        var fd: Int32 = -1
        while true {
            fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else { return .unreachable }
            let connected = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            if connected == 0 { break }
            let error = errno
            Darwin.close(fd)
            // No socket yet, or a stale one: the running copy may be between taking the lock and
            // listening. This process exits right after asking, so a short sleep costs nothing.
            guard error == ENOENT || error == ECONNREFUSED, Date() < deadline else { return .unreachable }
            usleep(50_000)
        }
        defer { Darwin.close(fd) }
        configure(fd, timeout: max(1, deadline.timeIntervalSinceNow))
        let line = command + "\n"
        let sent = line.withCString { write(fd, $0, strlen($0)) }
        guard sent == line.utf8.count else { return .refused }
        return readLine(fd, limit: 16).map { $0 + "\n" } == accepted ? .shown : .refused
    }

    // MARK: Plumbing

    /// A sockaddr_un for `path`, or an error when it does not fit (never a truncated path).
    static func address(_ path: String) throws -> sockaddr_un {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard !bytes.isEmpty, bytes.count < capacity, !bytes.contains(0) else {
            throw Failure(call: "sockaddr_un (path of \(bytes.count) bytes)", code: ENAMETOOLONG)
        }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            buffer.copyBytes(from: bytes)
            buffer[bytes.count] = 0
        }
        return address
    }

    /// No SIGPIPE, and reads and writes that give up after `timeout` seconds.
    private static func configure(_ fd: Int32, timeout: TimeInterval) {
        var on: Int32 = 1
        _ = setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        let seconds = timeout.rounded(.down)
        var limit = timeval(tv_sec: Int(seconds), tv_usec: Int32((timeout - seconds) * 1_000_000))
        _ = setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &limit, socklen_t(MemoryLayout<timeval>.size))
        _ = setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &limit, socklen_t(MemoryLayout<timeval>.size))
    }

    /// One line (without its newline), or nil on EOF, a timeout or `limit` bytes without one.
    private static func readLine(_ fd: Int32, limit: Int) -> String? {
        var bytes: [UInt8] = []
        var byte: UInt8 = 0
        while bytes.count < limit {
            guard read(fd, &byte, 1) == 1 else { return nil }
            if byte == UInt8(ascii: "\n") {
                return String(decoding: bytes, as: UTF8.self)
            }
            bytes.append(byte)
        }
        return nil
    }
}

/// Show requests on their way to the app delegate. One that arrives before the app can show a
/// window (it is still launching) waits here and is served when the delegate attaches.
@MainActor
final class ShowRequests {
    private var show: (() -> Void)?
    /// A request is waiting for `attach`.
    private(set) var waiting = false

    /// A request arrived.
    func arrived() {
        if let show {
            show()
        } else {
            waiting = true
        }
    }

    /// From now on, requests call `show`; one that is waiting does so now.
    func attach(_ show: @escaping () -> Void) {
        self.show = show
        if waiting {
            waiting = false
            show()
        }
    }

    /// The channel's `onShow`: hops from its queue to the main actor.
    nonisolated static func forwarder(to requests: ShowRequests) -> @Sendable () -> Void {
        {
            DispatchQueue.main.async {
                MainActor.assumeIsolated { requests.arrived() }
            }
        }
    }
}
