// Swift over the core's C ABI (InkCore, inkwell.h). The header holds the contract; in short:
//
// - Events arrive on ONE core thread ("ink-events"). `InkSession.start`'s handler runs there: hop
//   to the main actor for UI and return promptly. Never call `shutdown` from it.
// - Engines registered here are called on core WORKER threads, several calls at once. They answer
//   through their completion, from any thread, exactly once.
// - The core installs the only logger for its code: do not install one for the core's targets.
//   Event payloads can carry the user's words; never log them.
import Foundation
import InkCore
import Synchronization

/// A status the core returned (`INK_ERR_*` in inkwell.h).
public struct InkStatusError: Error, Equatable, CustomStringConvertible {
    /// The code.
    public let code: Int32

    public var description: String {
        switch code {
        case INK_ERR_NOT_INITIALIZED: "the core is not running"
        case INK_ERR_ALREADY_INITIALIZED: "the core is already running"
        case INK_ERR_INVALID_ARGUMENT: "the core could not read the argument"
        case INK_ERR_FAILED: "the core could not do it"
        case INK_ERR_UNKNOWN_CALL: "no such call is waiting"
        case INK_ERR_PANIC: "a bug in the core (contained)"
        default: "core status \(code)"
        }
    }
}

private func check(_ status: Int32) throws {
    if status != INK_OK {
        throw InkStatusError(code: status)
    }
}

/// `ink_init`'s configuration.
public struct InkConfig: Encodable, Sendable {
    /// The library, recordings and models live here (absolute).
    public var dataDir: String
    /// Models elsewhere (absolute), or nil for `<dataDir>/models`.
    public var modelsDir: String?
    /// off, error, warn, info, debug or trace; nil for info.
    public var logLevel: String?
    /// Whether the core writes log lines to stderr; nil for yes.
    public var logStderr: Bool?

    public init(dataDir: String, modelsDir: String? = nil, logLevel: String? = nil, logStderr: Bool? = nil) {
        self.dataDir = dataDir
        self.modelsDir = modelsDir
        self.logLevel = logLevel
        self.logStderr = logStderr
    }

    enum CodingKeys: String, CodingKey {
        case dataDir = "data_dir"
        case modelsDir = "models_dir"
        case logLevel = "log_level"
        case logStderr = "log_stderr"
    }
}

/// Holds the event handler for the event thread.
private final class EventSink: Sendable {
    let handler: @Sendable (InkEvent) -> Void

    init(_ handler: @escaping @Sendable (InkEvent) -> Void) {
        self.handler = handler
    }
}

/// The running core. One per process; `shutdown` before quitting (llama.cpp's Metal backend aborts
/// the process at exit if a model is still loaded).
public final class InkSession: Sendable {
    private let sink: EventSink
    private let stopped = Mutex(false)

    private init(sink: EventSink) {
        self.sink = sink
    }

    /// Starts the core. `onEvent` runs on the core's event thread, in order (see the file header).
    public static func start(
        _ config: InkConfig,
        onEvent: @escaping @Sendable (InkEvent) -> Void
    ) throws -> InkSession {
        let session = InkSession(sink: EventSink(onEvent))
        let json = String(decoding: try JSONEncoder().encode(config), as: UTF8.self)
        // Unretained: the session owns the sink, and `shutdown` (which it runs on deinit at the
        // latest) returns only once the event thread has stopped.
        let ctx = Unmanaged.passUnretained(session.sink).toOpaque()
        let status = json.withCString { cfg in
            ink_init(cfg, { ctx, json, len in
                guard let ctx, let json else { return }
                let sink = Unmanaged<EventSink>.fromOpaque(ctx).takeUnretainedValue()
                let data = Data(bytes: json, count: len)
                // Only JSON without a "type" fails to decode at all: a known event whose content
                // does not decode arrives as .undecodable with its type and record.
                sink.handler((try? InkEvent.decode(data)) ?? .undecodable(type: "", record: nil))
            }, ctx)
        }
        try check(status)
        return session
    }

    /// Queues a command (see inkwell.h for the commands). Its outcome arrives as events.
    public func command(_ json: String) throws {
        try check(json.withCString { ink_command($0) })
    }

    /// Queues a command built from `fields` (strings only; `cmd` among them).
    public func command(_ fields: [String: String]) throws {
        try command(String(decoding: try JSONEncoder().encode(fields), as: UTF8.self))
    }

    /// Registers an engine the shell owns. From here the router may call it on worker threads.
    public func register(_ engine: some InkOfflineEngine) throws {
        try InkEngineTable.register(engine)
    }

    /// The latest bands of the live audio, copied out. Any thread, any rate: it never blocks.
    public static func bands() -> InkBands {
        var out = InkBands()
        _ = ink_bands_read(&out)
        return out
    }

    /// Stops the core: every worker, every engine (their releases run first), every model. When
    /// it returns no event arrives any more. Safe to call twice. Never from the event handler.
    public func shutdown() {
        let first = stopped.withLock { stopped in
            defer { stopped = true }
            return !stopped
        }
        if first {
            _ = ink_shutdown()
        }
    }

    deinit {
        shutdown()
    }
}

/// A transcribed stretch, in ms from the start of the audio the engine was given.
public struct InkSegment: Sendable, Equatable {
    public var startMs: UInt64
    public var endMs: UInt64
    public var text: String

    public init(startMs: UInt64, endMs: UInt64, text: String) {
        self.startMs = startMs
        self.endMs = endMs
        self.text = text
    }
}

/// Why an engine did not answer with segments. There is no free text: the core reads a kind and
/// an optional code, so nothing an engine says can carry what it heard into a log or an event.
public enum InkEngineError: Error, Sendable, Equatable {
    /// It failed; `code` is the engine's own, shown in the core's error.
    case failed(code: Int)
    case cancelled
    case modelMissing
    /// The request could not be read (its samples or options).
    case badRequest
}

/// An offline engine the shell owns (dictation and meeting finals).
public protocol InkOfflineEngine: AnyObject, Sendable {
    /// A unique id.
    var id: String { get }
    /// The weights' licence.
    var licence: String { get }
    /// The jobs it fills (`.dictationFinal`, `.meetingFinal`) with its measured word error rate.
    var jobs: [(job: Job, wer: Double)] { get }

    /// Called on a core worker thread with 16 kHz mono samples (gain applied). Call `completion`
    /// exactly once, from any thread, now or later.
    func transcribe(
        _ samples: [Float],
        channel: Channel,
        context: String?,
        completion: @escaping @Sendable (Result<[InkSegment], InkEngineError>) -> Void
    )
}

/// The engine behind a table's `ctx`.
private final class EngineBox: Sendable {
    let engine: any InkOfflineEngine

    init(_ engine: any InkOfflineEngine) {
        self.engine = engine
    }
}

private struct Options: Decodable {
    let channel: Channel
    let context: String?
}

/// `info_json`, as the header gives it.
private struct EngineInfo: Encodable {
    struct Score: Encodable {
        let job: String
        let wer: Double
    }

    let id: String
    let licence: String
    let jobs: [Score]
}

private enum InkEngineTable {
    static func register(_ engine: any InkOfflineEngine) throws {
        let info = EngineInfo(
            id: engine.id,
            licence: engine.licence,
            jobs: engine.jobs.map { EngineInfo.Score(job: $0.job.rawValue, wer: $0.wer) }
        )
        let infoJSON = String(decoding: try JSONEncoder().encode(info), as: UTF8.self)
        // Retained for the core; its release function balances this.
        let ctx = Unmanaged.passRetained(EngineBox(engine)).toOpaque()
        let status = infoJSON.withCString { infoPtr in
            var table = InkEngineVTable(
                size: UInt32(MemoryLayout<InkEngineVTable>.size),
                kind: INK_ENGINE_OFFLINE,
                info_json: infoPtr,
                ctx: ctx,
                transcribe: { ctx, call, samples, len, options in
                    guard let ctx else { return }
                    let engine = Unmanaged<EngineBox>.fromOpaque(ctx).takeUnretainedValue().engine
                    // Valid only during this call: copied before anything answers later.
                    let audio = samples.map { Array(UnsafeBufferPointer(start: $0, count: len)) } ?? []
                    let parsed = options.flatMap { try? JSONDecoder().decode(Options.self, from: Data(String(cString: $0).utf8)) }
                    engine.transcribe(audio, channel: parsed?.channel ?? .mic, context: parsed?.context) { result in
                        InkEngineTable.complete(call, result)
                    }
                },
                cancel: nil,
                release: { ctx in
                    guard let ctx else { return }
                    Unmanaged<EngineBox>.fromOpaque(ctx).release()
                }
            )
            return ink_register_engine(&table)
        }
        if status != INK_OK {
            // A refused table is never released by the core.
            Unmanaged<EngineBox>.fromOpaque(ctx).release()
            throw InkStatusError(code: status)
        }
    }

    static func complete(_ call: UInt64, _ result: Result<[InkSegment], InkEngineError>) {
        let object: [String: Any]
        switch result {
        case .success(let segments):
            object = ["segments": segments.map {
                ["start_ms": $0.startMs, "end_ms": $0.endMs, "text": $0.text] as [String: Any]
            }]
        case .failure(let error):
            let fields: [String: Any] = switch error {
            case .failed(let code): ["kind": "failed", "code": code]
            case .cancelled: ["kind": "cancelled"]
            case .modelMissing: ["kind": "model_missing"]
            case .badRequest: ["kind": "bad_request"]
            }
            object = ["error": fields]
        }
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
        // INK_ERR_UNKNOWN_CALL means the core gave up on this call (cancelled, shut down): fine.
        _ = String(decoding: data, as: UTF8.self).withCString { ink_engine_complete(call, $0) }
    }
}
