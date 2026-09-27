// Test doubles for the Apple engines: a scripted decoder that knows where each window sits in the
// stream, and a sink that records what a stream sends.
import AppleEngines
import Foundation
import InkBridge
import Synchronization

/// A spoken word of a script, in stream samples.
struct ScriptWord {
    var text: String
    var start: Int
    var end: Int
}

/// Unbroken speech: `count` words of 0.25 s with 0.05 s between them, from `from`.
func unbroken(_ count: Int, from: Int = 8_000, prefix: String = "w") -> [ScriptWord] {
    (0..<count).map { i in
        let start = from + i * 4_800
        return ScriptWord(text: "\(prefix)\(i)", start: start, end: start + 4_000)
    }
}

/// Stream audio whose every sample is its own index, so a decoder handed a window knows where the
/// window sits in the stream (exact in Float up to 2^24 samples: 17 minutes).
func indexed(_ range: Range<Int>) -> [Float] {
    range.map { Float($0) }
}

/// Decodes a window of `indexed` audio as the words of `script` fully inside it.
struct ScriptedDecoder: WindowDecoder {
    let script: [ScriptWord]
    var delay: Duration = .zero
    let calls = Counter()

    func decode(_ samples: [Float]) async throws(ParakeetError) -> DecodedWindow {
        calls.add()
        if delay > .zero { try? await Task.sleep(for: delay) }
        guard let first = samples.first else { return DecodedWindow(words: []) }
        let start = Int(first)
        let end = start + samples.count
        return DecodedWindow(words: script.filter { $0.start >= start && $0.end <= end }.map {
            TimedWord(text: $0.text, start: $0.start - start, end: $0.end - start)
        })
    }
}

/// A thread-safe count.
final class Counter: Sendable {
    private let n = Mutex(0)
    func add() { n.withLock { $0 += 1 } }
    var value: Int { n.withLock { $0 } }
}

/// What a stream sent, in order.
final class RecordingSink: LiveSink {
    enum Sent: Equatable {
        case partial(String)
        case final(InkSegment)
        case stalled(Int)
    }

    private let sent = Mutex<[Sent]>([])

    func partial(_ text: String) { sent.withLock { $0.append(.partial(text)) } }
    func final(_ segment: InkSegment) { sent.withLock { $0.append(.final(segment)) } }
    func stalled(code: Int) { sent.withLock { $0.append(.stalled(code)) } }

    var all: [Sent] { sent.withLock { $0 } }
    var finals: [InkSegment] { all.compactMap { if case .final(let s) = $0 { s } else { nil } } }
    var partials: [String] { all.compactMap { if case .partial(let p) = $0 { p } else { nil } } }

    /// Waits until `done` holds for what was sent; looks at least once.
    func wait(_ timeout: TimeInterval = 5, until done: ([Sent]) -> Bool) -> Bool {
        let until = Date().addingTimeInterval(timeout)
        while !done(all) {
            if Date() >= until { return false }
            Thread.sleep(forTimeInterval: 0.005)
        }
        return true
    }
}

/// A one-shot signal for a completion.
final class Signal<T: Sendable>: Sendable {
    private let value = Mutex<T?>(nil)
    func set(_ v: T) { value.withLock { $0 = v } }
    func wait(_ timeout: TimeInterval = 5) -> T? {
        let until = Date().addingTimeInterval(timeout)
        while true {
            if let v = value.withLock({ $0 }) { return v }
            if Date() >= until { return nil }
            Thread.sleep(forTimeInterval: 0.005)
        }
    }
}

/// Every event a session delivered, in order.
final class EventLog: Sendable {
    private let events = Mutex<[InkEvent]>([])

    func record(_ event: InkEvent) { events.withLock { $0.append(event) } }
    var all: [InkEvent] { events.withLock { $0 } }

    /// Waits for the first event `pick` accepts; looks at least once.
    func wait<T>(_ timeout: TimeInterval, _ pick: (InkEvent) -> T?) -> T? {
        let until = Date().addingTimeInterval(timeout)
        while true {
            if let found = all.lazy.compactMap(pick).first { return found }
            if Date() >= until { return nil }
            Thread.sleep(forTimeInterval: 0.01)
        }
    }
}

/// A started core over a fresh data directory, removed by `stop`.
struct TestCore {
    let session: InkSession
    let events: EventLog
    let data: URL

    static func start() throws -> TestCore {
        let data = FileManager.default.temporaryDirectory
            .appendingPathComponent("inkwell-apple-engines-\(UUID().uuidString)")
        let events = EventLog()
        let session = try InkSession.start(
            InkConfig(dataDir: data.path, logLevel: "warn"), onEvent: { events.record($0) })
        _ = events.wait(5) { if case .coreReady = $0 { true } else { nil } }
        return TestCore(session: session, events: events, data: data)
    }

    func stop() {
        session.shutdown()
        try? FileManager.default.removeItem(at: data)
    }

    var registered: [EngineRegistered] {
        events.all.compactMap { if case .engineRegistered(let e) = $0 { e } else { nil } }
    }
}

/// The pthread name of the calling thread (the core names its threads).
func currentThreadName() -> String {
    var buffer = [CChar](repeating: 0, count: 64)
    pthread_getname_np(pthread_self(), &buffer, buffer.count)
    return String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
}

/// The repository's fixtures directory.
let fixtures = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()  // AppleEnginesTests
    .deletingLastPathComponent()  // Tests
    .deletingLastPathComponent()  // mac
    .deletingLastPathComponent()  // the repository
    .appendingPathComponent("fixtures")
