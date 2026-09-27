// Live partials from Parakeet (the live-partials job on the Mac), registered with the core as a
// streaming engine.
//
// Threads, per the header's contract:
// - `openStream`, `push`, `finish` and `close` arrive on a core worker thread. `push` only appends
//   to the stream's buffer under its lock and returns; it never waits for a decode.
// - Decoding runs in one task per stream at a time. When it finishes, it takes the newest window
//   if a hop of audio has arrived meanwhile, so a slow decode skips hypotheses (partials are
//   ephemeral) rather than queueing them. A backlog of `stallAfter` is reported as a stall once.
// - Events leave through the stream's sink one at a time (its own lock), in the order they
//   happened; after `close` none are sent.

import Foundation
import InkBridge
import Synchronization

/// Where a stream's words go: the core (`InkStreamEvents`), or a test.
public protocol LiveSink: Sendable {
    func partial(_ text: String)
    func final(_ segment: InkSegment)
    func stalled(code: Int)
}

/// The core, through `ink_stream_event`.
struct CoreSink: LiveSink {
    let events: InkStreamEvents

    func partial(_ text: String) { events.partial(text) }
    func final(_ segment: InkSegment) { events.final(segment) }
    func stalled(code: Int) { events.stalled(code: code) }
}

/// A stream, for an observer: its number in this engine and its side.
public struct StreamTag: Sendable, Hashable {
    public var number: Int
    public var channel: Channel
}

/// One decode and what it sent: for measuring latency and flicker (the harness), not for the app.
public struct LiveDecode: Sendable {
    /// The stream samples decoded.
    public var window: Range<Int>
    /// The words, in samples from `window.lowerBound`.
    public var decoded: DecodedWindow
    /// What was sent.
    public var outputs: [LiveOutput]
    /// How long the decode took.
    public var decodeTime: Duration
    /// When the outputs were sent.
    public var sentAt: ContinuousClock.Instant
    /// Whether this was the stream's last decode, at `finish`.
    public var isLast: Bool
}

/// Watches streams: every push and every decode.
public protocol LiveObserver: Sendable {
    func pushed(_ stream: StreamTag, upTo sample: Int, at: ContinuousClock.Instant)
    func decoded(_ stream: StreamTag, _ decode: LiveDecode)
}

/// Parakeet's live partials, as the core's streaming engine.
public final class ParakeetLiveEngine: InkStreamingEngine {
    public let id: String
    /// Parakeet TDT 0.6B v3's weights (NVIDIA, through FluidInference's Core ML conversion).
    public let licence = "CC-BY-4.0"
    public let wer: Double
    let decoder: any WindowDecoder
    let config: LiveWindowConfig
    let observer: (any LiveObserver)?
    private let streams = Atomic<Int>(0)

    /// `wer` is the word error rate measured for this model on AMI IHM when the engines were chosen
    /// (23.4).
    public init(
        decoder: any WindowDecoder = ParakeetModel.shared,
        id: String = "fluidaudio-parakeet-tdt-0.6b-v3",
        wer: Double = 23.4,
        config: LiveWindowConfig = LiveWindowConfig(),
        observer: (any LiveObserver)? = nil
    ) {
        self.decoder = decoder
        self.id = id
        self.wer = wer
        self.config = config
        self.observer = observer
    }

    public func openStream(channel: Channel, events: InkStreamEvents) throws(InkEngineError) -> any InkLiveStream {
        stream(channel: channel, sink: CoreSink(events: events))
    }

    /// A stream sending to `sink` (tests use their own).
    public func stream(channel: Channel, sink: any LiveSink) -> ParakeetLiveStream {
        let number = streams.wrappingAdd(1, ordering: .relaxed).newValue
        return ParakeetLiveStream(
            decoder: decoder, config: config, sink: sink, observer: observer,
            tag: StreamTag(number: number, channel: channel))
    }
}

/// One side's live stream. See the file header for its threads.
public final class ParakeetLiveStream: InkLiveStream {
    /// A backlog this long (3 s) while a decode runs is reported as a stall.
    public static let stallAfter = 48_000

    private struct State {
        var window: LiveWindow
        var decoding = false
        var stalled = false
        var closed = false
        var failure: InkEngineError?
        var finishing: (@Sendable (Result<Void, InkEngineError>) -> Void)?
    }

    private let decoder: any WindowDecoder
    private let config: LiveWindowConfig
    private let sink: any LiveSink
    private let observer: (any LiveObserver)?
    private let tag: StreamTag
    private let state: Mutex<State>
    /// Held while sending, so the sink hears one event at a time, in order.
    private let sending = Mutex(())

    init(decoder: any WindowDecoder, config: LiveWindowConfig, sink: any LiveSink,
         observer: (any LiveObserver)?, tag: StreamTag) {
        self.decoder = decoder
        self.config = config
        self.sink = sink
        self.observer = observer
        self.tag = tag
        state = Mutex(State(window: LiveWindow(config: config)))
    }

    public func push(_ samples: [Float]) throws(InkEngineError) {
        let (window, stall, received): (Window?, Bool, Int) = try state.withLock { s throws(InkEngineError) in
            if let failure = s.failure { throw failure }
            // After finish or close the core sends no audio; audio that came anyway is refused.
            guard !s.closed, s.finishing == nil else { throw InkEngineError.badRequest }
            s.window.append(samples)
            if !s.decoding, s.window.wantsDecode {
                s.decoding = true
                return (s.window.takeWindow(), false, s.window.received)
            }
            let stall = s.decoding && !s.stalled && s.window.backlog >= Self.stallAfter
            if stall { s.stalled = true }
            return (nil, stall, s.window.received)
        }
        observer?.pushed(tag, upTo: received, at: .now)
        if stall {
            Log.engine.notice("live partials: 3 s of audio wait for a decode; reported as a stall")
            send { $0.stalled(code: 1) }
        }
        if let window {
            Task.detached(priority: .userInitiated) { await self.run(window) }
        }
    }

    public func finish(completion: @escaping @Sendable (Result<Void, InkEngineError>) -> Void) {
        enum Then { case answer(Result<Void, InkEngineError>), flush, wait }
        let then: Then = state.withLock { s in
            if let failure = s.failure { return .answer(.failure(failure)) }
            guard !s.closed, s.finishing == nil else { return .answer(.failure(.badRequest)) }
            s.finishing = completion
            if s.decoding { return .wait }  // the running decode flushes when it is done
            s.decoding = true
            return .flush
        }
        switch then {
        case .answer(let result): completion(result)
        case .flush: Task.detached(priority: .userInitiated) { await self.flush() }
        case .wait: break
        }
    }

    public func close() {
        let waiting = state.withLock { s in
            s.closed = true
            // The audio is let go of now; a decode still running finds the stream closed.
            s.window = LiveWindow(config: config)
            defer { s.finishing = nil }
            return s.finishing
        }
        // A send under way finishes first; every later one sees the stream closed. So nothing is
        // sent once this returns.
        sending.withLock { _ in }
        // The core stopped waiting for this finish when it closed the stream.
        waiting?(.failure(.cancelled))
    }

    /// Decodes `first`, then the newest window for as long as a hop of audio arrived meanwhile.
    private func run(_ first: Window) async {
        var window = first
        while true {
            let started = ContinuousClock.now
            let result: Result<DecodedWindow, ParakeetError>
            do throws(ParakeetError) {
                result = .success(try await decoder.decode(window.samples))
            } catch {
                result = .failure(error)
            }
            enum Next { case decode(Window), flush, stop }
            let (outputs, next): ([LiveOutput], Next) = state.withLock { s in
                if s.closed {
                    s.decoding = false
                    return ([], .stop)
                }
                guard case .success(let decoded) = result else {
                    s.failure = Self.engineError(result)
                    // A finish waiting is answered by the flush, which reports the failure.
                    if s.finishing != nil { return ([], .flush) }
                    s.decoding = false
                    return ([], .stop)
                }
                let out = s.window.apply(decoded, of: window)
                if s.window.backlog < config.hop { s.stalled = false }
                if s.finishing != nil { return (out, .flush) }
                if s.window.wantsDecode { return (out, .decode(s.window.takeWindow())) }
                s.decoding = false
                return (out, .stop)
            }
            if case .success(let decoded) = result {
                emit(outputs, decoded: decoded, window: window, started: started, isLast: false)
            } else {
                Log.engine.error("live partials: a decode failed; the stream ends")
            }
            switch next {
            case .decode(let w): window = w
            case .flush: return await flush()
            case .stop: return
            }
        }
    }

    /// The end of the stream: decodes what is left, settles all of it, then answers `finish`.
    private func flush() async {
        let (window, failure): (Window?, InkEngineError?) = state.withLock { s in
            (s.failure == nil && !s.closed ? s.window.takeLastWindow() : nil, s.failure)
        }
        var outcome: Result<Void, InkEngineError> = failure.map { .failure($0) } ?? .success(())
        if let window, failure == nil {
            let started = ContinuousClock.now
            do throws(ParakeetError) {
                let decoded = try await decoder.decode(window.samples)
                let outputs = state.withLock { s in s.closed ? [] : s.window.applyLast(decoded, of: window) }
                emit(outputs, decoded: decoded, window: window, started: started, isLast: true)
            } catch {
                outcome = .failure(Self.engineError(.failure(error)))
            }
        } else if failure == nil {
            // Too little left to decode: the partial is cleared, nothing is settled.
            let outputs = state.withLock { s in s.closed ? [] : s.window.endWithoutWindow() }
            emit(outputs, decoded: DecodedWindow(words: []), window: nil, started: .now, isLast: true)
        }
        let completion = state.withLock { s in
            s.decoding = false
            defer { s.finishing = nil }
            return s.finishing
        }
        completion?(outcome)
    }

    private func emit(
        _ outputs: [LiveOutput], decoded: DecodedWindow, window: Window?,
        started: ContinuousClock.Instant, isLast: Bool
    ) {
        send { sink in
            for output in outputs {
                switch output {
                case .partial(let text): sink.partial(text)
                case .final(let segment): sink.final(segment)
                }
            }
        }
        guard let observer else { return }
        let now = ContinuousClock.now
        let range = window.map { $0.start..<$0.end } ?? 0..<0
        observer.decoded(tag, LiveDecode(
            window: range, decoded: decoded, outputs: outputs, decodeTime: now - started,
            sentAt: now, isLast: isLast))
    }

    private func send(_ body: (any LiveSink) -> Void) {
        sending.withLock { _ in
            guard !state.withLock({ $0.closed }) else { return }
            body(sink)
        }
    }

    private static func engineError(_ result: Result<DecodedWindow, ParakeetError>) -> InkEngineError {
        guard case .failure(let error) = result else { return .failed(code: 0) }
        return error.engineError
    }
}

extension ParakeetError {
    /// As the core reads it: a kind and a code.
    var engineError: InkEngineError {
        switch self {
        case .modelMissing: .modelMissing
        case .downloadRefused: .modelMissing
        case .unsupported: .unavailable(code: 1)
        case .loadFailed(let code): .failed(code: 100 + code)
        case .decodeFailed(let code): .failed(code: 200 + code)
        }
    }
}
