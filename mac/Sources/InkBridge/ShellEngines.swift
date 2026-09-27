// Live-stream engines and language models the shell registers with the core (INK_ENGINE_STREAMING
// and INK_ENGINE_LLM in inkwell.h). The threading contract is the header's:
//
// - Every function here is called on a core WORKER thread, never the main thread. A stream's calls
//   come from one thread at a time, in order; different streams and different generations may run
//   at once.
// - Each step is answered exactly once through its completion (this file answers the core), from
//   any thread. `push` answers as soon as the samples are copied: recognition runs elsewhere.
// - A stream's words go back through its `InkStreamEvents`, from any thread, one at a time per
//   stream, in order.
// - Errors carry a kind and a code, never text: an engine's words could quote what it heard.
import Foundation
import InkCore
import Synchronization

// MARK: - Live streams

/// A live stream's way back to the core. Cheap to copy; sending only queues.
public struct InkStreamEvents: Sendable {
    /// The core's number for the stream.
    public let stream: UInt64

    public init(stream: UInt64) {
        self.stream = stream
    }

    /// The words not settled yet. Each partial replaces the last; an empty one clears it. Never
    /// stored. Returns false once the core has closed the stream.
    @discardableResult
    public func partial(_ text: String) -> Bool {
        send(["partial": text])
    }

    /// Settled words, in ms from the stream's first sample: only this final's own words, never the
    /// stream's text so far. Returns false once the core has closed the stream.
    @discardableResult
    public func final(_ segment: InkSegment) -> Bool {
        send(["final": ["start_ms": segment.startMs, "end_ms": segment.endMs, "text": segment.text] as [String: Any]])
    }

    /// The engine has fallen behind real time. `code` is the engine's own; no text is sent.
    @discardableResult
    public func stalled(code: Int? = nil) -> Bool {
        send(["stalled": code.map { ["code": $0] } ?? [String: Int]()])
    }

    private func send(_ object: [String: Any]) -> Bool {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return false }
        return String(decoding: data, as: UTF8.self).withCString { ink_stream_event(stream, $0) } == INK_OK
    }
}

/// An engine that turns live audio into partials and finals (the live-partials job).
public protocol InkStreamingEngine: AnyObject, Sendable {
    /// A unique id, across every kind of engine.
    var id: String { get }
    /// The weights' licence.
    var licence: String { get }
    /// Its measured word error rate on live partials, percent: the router picks the lowest.
    var wer: Double { get }

    /// Core worker thread. Opens a stream for one side; its words go to `events`. Throw to refuse
    /// (the core still closes it).
    func openStream(channel: Channel, events: InkStreamEvents) throws(InkEngineError) -> any InkLiveStream
}

/// One open stream.
public protocol InkLiveStream: AnyObject, Sendable {
    /// Core worker thread, in order: the next 16 kHz mono samples, gain applied. Keep them and
    /// return at once (the core gives up after 2 s). Throwing ends the stream.
    func push(_ samples: [Float]) throws(InkEngineError)

    /// No more audio: recognise the rest, send every trailing event, then call `completion` once,
    /// from any thread (the core gives up after 30 s).
    func finish(completion: @escaping @Sendable (Result<Void, InkEngineError>) -> Void)

    /// The core is done with the stream, after `finish` answered or instead of it. Free it; send
    /// nothing more. Called exactly once.
    func close()
}

/// The engine behind a streaming table's `ctx`, and its open streams by number.
private final class StreamingBox: Sendable {
    let engine: any InkStreamingEngine
    let streams = Mutex<[UInt64: any InkLiveStream]>([:])

    init(_ engine: any InkStreamingEngine) {
        self.engine = engine
    }

    func stream(_ number: UInt64) -> (any InkLiveStream)? {
        streams.withLock { $0[number] }
    }

    static func from(_ ctx: UnsafeMutableRawPointer?) -> StreamingBox? {
        ctx.map { Unmanaged<StreamingBox>.fromOpaque($0).takeUnretainedValue() }
    }
}

enum StreamingTable {
    static func register(_ engine: any InkStreamingEngine) throws {
        let info = EngineInfo(id: engine.id, licence: engine.licence, jobs: [.init(job: Job.livePartials.rawValue, wer: engine.wer)])
        let infoJSON = String(decoding: try JSONEncoder().encode(info), as: UTF8.self)
        // Retained for the core; its release function balances this.
        let ctx = Unmanaged.passRetained(StreamingBox(engine)).toOpaque()
        let status = infoJSON.withCString { infoPtr in
            var table = InkEngineVTable()
            table.size = UInt32(MemoryLayout<InkEngineVTable>.size)
            table.kind = INK_ENGINE_STREAMING
            table.info_json = infoPtr
            table.ctx = ctx
            table.release = { ctx in
                guard let ctx else { return }
                Unmanaged<StreamingBox>.fromOpaque(ctx).release()
            }
            table.stream_open = { ctx, call, number, options in
                guard let box = StreamingBox.from(ctx) else { return StreamingTable.answer(call, .failure(.badRequest)) }
                guard let channel = StreamOptions.channel(options) else {
                    return StreamingTable.answer(call, .failure(.badRequest))
                }
                do throws(InkEngineError) {
                    let stream = try box.engine.openStream(channel: channel, events: InkStreamEvents(stream: number))
                    box.streams.withLock { $0[number] = stream }
                    StreamingTable.answer(call, .success(()))
                } catch {
                    StreamingTable.answer(call, .failure(error))
                }
            }
            table.stream_push = { ctx, call, number, samples, len in
                guard let stream = StreamingBox.from(ctx)?.stream(number), samples != nil || len == 0 else {
                    return StreamingTable.answer(call, .failure(.badRequest))
                }
                // Copied: the pointer is valid only during this call.
                let audio = samples.map { Array(UnsafeBufferPointer(start: $0, count: len)) } ?? []
                do throws(InkEngineError) {
                    try stream.push(audio)
                    StreamingTable.answer(call, .success(()))
                } catch {
                    StreamingTable.answer(call, .failure(error))
                }
            }
            table.stream_finish = { ctx, call, number in
                guard let stream = StreamingBox.from(ctx)?.stream(number) else {
                    return StreamingTable.answer(call, .failure(.badRequest))
                }
                stream.finish { StreamingTable.answer(call, $0) }
            }
            table.stream_close = { ctx, number in
                let stream = StreamingBox.from(ctx)?.streams.withLock { $0.removeValue(forKey: number) }
                stream?.close()
            }
            return ink_register_engine(&table)
        }
        if status != INK_OK {
            // A refused table is never released by the core.
            Unmanaged<StreamingBox>.fromOpaque(ctx).release()
            throw InkStatusError(code: status)
        }
    }

    static func answer(_ call: UInt64, _ result: Result<Void, InkEngineError>) {
        switch result {
        case .success: InkAnswer.send(call, ["ok": true])
        case .failure(let error): InkAnswer.send(call, ["error": error.fields])
        }
    }
}

/// A stream's options: `{"channel":"mic"|"far"}`. Nothing is guessed: the channel decides "you"
/// versus "them".
enum StreamOptions {
    private struct Options: Decodable {
        let channel: Channel
    }

    static func channel(_ options: UnsafePointer<CChar>?) -> Channel? {
        guard let options else { return nil }
        return try? JSONDecoder().decode(Options.self, from: Data(bytes: options, count: strlen(options))).channel
    }
}

// MARK: - Language models

/// One generation request, as the core sends it. It holds the user's words: never log it.
public struct InkLlmRequest: Sendable, Equatable, Decodable {
    public var system: String
    public var user: String
    public var maxTokens: Int
    public var temperature: Double
    /// A JSON schema the answer must match, for structured tasks.
    public var jsonSchema: String?

    public init(system: String, user: String, maxTokens: Int, temperature: Double, jsonSchema: String? = nil) {
        self.system = system
        self.user = user
        self.maxTokens = maxTokens
        self.temperature = temperature
        self.jsonSchema = jsonSchema
    }

    enum CodingKeys: String, CodingKey {
        case system, user, temperature
        case maxTokens = "max_tokens"
        case jsonSchema = "json_schema"
    }
}

/// Whether the core still wants a call's answer. Thread-safe.
public final class InkCancellation: Sendable {
    private let state = Mutex<(cancelled: Bool, handlers: [@Sendable () -> Void])>((false, []))

    public init() {}

    /// Whether the core no longer wants the answer.
    public var isCancelled: Bool { state.withLock { $0.cancelled } }

    /// Runs `handler` once when the call is cancelled: now, if it already is.
    public func onCancel(_ handler: @escaping @Sendable () -> Void) {
        let now = state.withLock { state -> Bool in
            if state.cancelled { return true }
            state.handlers.append(handler)
            return false
        }
        if now { handler() }
    }

    /// Cancels the call and runs its handlers, once.
    public func cancel() {
        let handlers = state.withLock { state -> [@Sendable () -> Void] in
            guard !state.cancelled else { return [] }
            state.cancelled = true
            defer { state.handlers = [] }
            return state.handlers
        }
        handlers.forEach { $0() }
    }
}

/// A language model the shell owns (dictation polish).
public protocol InkLanguageModel: AnyObject, Sendable {
    /// A unique id, across every kind of engine.
    var id: String { get }
    /// The weights' licence.
    var licence: String { get }
    /// The model's name.
    var model: String { get }
    /// Whether the text stays on this Mac. Local-only mode refuses a model that says false.
    var isLocal: Bool { get }
    /// How many tokens its context holds, prompt and answer together, when it knows (the core
    /// sizes a meeting's summary and Ask to fit). Nil by default: the core then assumes 4,096.
    var contextTokens: Int? { get }

    /// Core worker thread. Generates an answer and calls `completion` once, from any thread. A
    /// model that cannot run now answers `.unavailable`, never made-up text.
    func generate(
        _ request: InkLlmRequest,
        cancellation: InkCancellation,
        completion: @escaping @Sendable (Result<String, InkEngineError>) -> Void
    )
}

extension InkLanguageModel {
    public var contextTokens: Int? { nil }
}

/// The model behind a language model's table `ctx`, and its calls in flight.
private final class ModelBox: Sendable {
    let model: any InkLanguageModel
    let calls = Mutex<[UInt64: InkCancellation]>([:])

    init(_ model: any InkLanguageModel) {
        self.model = model
    }

    static func from(_ ctx: UnsafeMutableRawPointer?) -> ModelBox? {
        ctx.map { Unmanaged<ModelBox>.fromOpaque($0).takeUnretainedValue() }
    }
}

private struct ModelInfo: Encodable {
    let id: String
    let licence: String
    let model: String
    let local: Bool
    /// Left out when nil (the core refuses a null).
    let contextTokens: Int?

    enum CodingKeys: String, CodingKey {
        case id, licence, model, local
        case contextTokens = "context_tokens"
    }
}

enum ModelTable {
    static func register(_ model: any InkLanguageModel) throws {
        let info = ModelInfo(
            id: model.id, licence: model.licence, model: model.model, local: model.isLocal,
            // The core takes 256 tokens or more; a smaller answer is a model that does not know.
            contextTokens: model.contextTokens.flatMap { $0 >= 256 ? $0 : nil })
        let infoJSON = String(decoding: try JSONEncoder().encode(info), as: UTF8.self)
        let ctx = Unmanaged.passRetained(ModelBox(model)).toOpaque()
        let status = infoJSON.withCString { infoPtr in
            var table = InkEngineVTable()
            table.size = UInt32(MemoryLayout<InkEngineVTable>.size)
            table.kind = INK_ENGINE_LLM
            table.info_json = infoPtr
            table.ctx = ctx
            table.release = { ctx in
                guard let ctx else { return }
                Unmanaged<ModelBox>.fromOpaque(ctx).release()
            }
            table.cancel = { ctx, call in
                let cancellation = ModelBox.from(ctx)?.calls.withLock { $0[call] }
                cancellation?.cancel()
            }
            table.generate = { ctx, call, request in
                guard let box = ModelBox.from(ctx), let request,
                      let parsed = try? JSONDecoder().decode(
                          InkLlmRequest.self, from: Data(bytes: request, count: strlen(request)))
                else {
                    return InkAnswer.send(call, ["error": InkEngineError.badRequest.fields])
                }
                let cancellation = InkCancellation()
                box.calls.withLock { $0[call] = cancellation }
                box.model.generate(parsed, cancellation: cancellation) { result in
                    box.calls.withLock { _ = $0.removeValue(forKey: call) }
                    switch result {
                    case .success(let text): InkAnswer.send(call, ["text": text])
                    case .failure(let error): InkAnswer.send(call, ["error": error.fields])
                    }
                }
            }
            return ink_register_engine(&table)
        }
        if status != INK_OK {
            Unmanaged<ModelBox>.fromOpaque(ctx).release()
            throw InkStatusError(code: status)
        }
    }
}

// MARK: - Answers

enum InkAnswer {
    /// Answers `call`. INK_ERR_UNKNOWN_CALL means the core gave up on it (cancelled, timed out,
    /// shut down): nothing to do.
    static func send(_ call: UInt64, _ object: [String: Any]) {
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
        _ = String(decoding: data, as: UTF8.self).withCString { ink_engine_complete(call, $0) }
    }
}

extension InkEngineError {
    /// The header's error object: a kind and an optional code, no text.
    var fields: [String: Any] {
        switch self {
        case .failed(let code): ["kind": "failed", "code": code]
        case .cancelled: ["kind": "cancelled"]
        case .modelMissing: ["kind": "model_missing"]
        case .badRequest: ["kind": "bad_request"]
        case .unavailable(let code): ["kind": "unavailable", "code": code]
        }
    }
}
