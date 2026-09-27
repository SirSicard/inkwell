// Parakeet TDT v3 on FluidAudio (Core ML, the Neural Engine): loaded once per process, never
// downloaded here, and called one decode at a time.
//
// - Loaded once: compiling the Core ML models takes seconds, so every stream and every caller
//   shares one `ParakeetModel` (`shared`). Callers that ask while it loads wait for that load.
// - Never downloaded here: `ModelHub.offlineMode` is set before FluidAudio's loader is touched,
//   so a missing or damaged file is reported (`modelMissing`, `downloadRefused`), and FluidAudio
//   neither fetches nor deletes its cache. Getting the models onto the Mac is a download the user
//   agrees to, made elsewhere.
// - Every time is counted in samples. FluidAudio's `ASRResult.duration` is 0 for audio longer than
//   one 15 s model window (its chunked path passes no samples to the result), so it is never read.
// - Every decode is bounded (`decodeLimit`). One that never returned used to keep the Neural
//   Engine's turn forever: every stream and the offline fallback waited behind it while every call
//   into the engine still answered. Now its caller gets `decodeTimedOut` and the next one the turn.
//   The abandoned decode may still be running on the Neural Engine: nothing can stop a Core ML
//   prediction under way. If it still holds FluidAudio's `AsrManager` (an actor), later decodes can
//   queue behind it and time out too, each after its own limit, so callers see failures instead
//   of waiting forever.

import FluidAudio
import Foundation
import Synchronization

/// Why Parakeet could not load or decode. Codes, never text: FluidAudio's messages can name paths.
public enum ParakeetError: Error, Sendable, Equatable {
    /// The models are not on this Mac.
    case modelMissing
    /// FluidAudio tried to fetch a file, and offline mode refused it.
    case downloadRefused
    /// Not an Apple Silicon Mac: the models are built for the Neural Engine.
    case unsupported
    /// Loading failed.
    case loadFailed(code: Int)
    /// A decode failed.
    case decodeFailed(code: Int)
    /// A decode did not return within its limit (`ParakeetModel.decodeLimit`).
    case decodeTimedOut
}

/// A decode as FluidAudio returns it, reduced to what the engine uses.
public struct Transcribed: Sendable, Equatable {
    /// The whole text, as FluidAudio writes it.
    public var text: String
    /// The words, in samples from the start of the audio decoded.
    public var words: [TimedWord]
    /// `ASRResult.duration`, kept only so a test can show it is not to be trusted.
    public var reportedDuration: Double

    public init(text: String, words: [TimedWord], reportedDuration: Double) {
        self.text = text
        self.words = words
        self.reportedDuration = reportedDuration
    }
}

/// What transcribes a buffer: FluidAudio's `AsrManager`, or a test's fake.
public protocol ParakeetBackend: Sendable {
    /// 16 kHz mono audio of at least 0.3 s. Long audio is chunked by FluidAudio.
    func transcribe(_ samples: [Float]) async throws(ParakeetError) -> Transcribed
}

/// Decodes a window of the live stream.
public protocol WindowDecoder: Sendable {
    func decode(_ samples: [Float]) async throws(ParakeetError) -> DecodedWindow
}

/// The process's Parakeet: loaded once, one decode at a time.
public actor ParakeetModel: WindowDecoder {
    /// Loads a backend. Runs at most once per successful load.
    public typealias Loader = @Sendable () async throws(ParakeetError) -> any ParakeetBackend

    /// The one every engine of the app shares.
    public static let shared = ParakeetModel(loader: FluidAudioParakeet.load)

    private let loader: Loader
    private var backend: (any ParakeetBackend)?
    private var loading: Task<Result<any ParakeetBackend, ParakeetError>, Never>?
    /// How many times the loader has run: 1 once loaded, whoever asked and how often.
    public private(set) var loads = 0
    /// Decodes wait here, so the Neural Engine takes one at a time and the streams share it
    /// in turn.
    private let turn = Turn()

    /// How long a decode of so many samples may take (`decodeLimit(samples:)`; tests shorten it).
    private let limit: @Sendable (Int) -> Duration

    public init(loader: @escaping Loader, decodeLimit: @escaping @Sendable (Int) -> Duration = ParakeetModel.decodeLimit) {
        self.loader = loader
        self.limit = decodeLimit
    }

    /// How long one decode of `samples` may run, from the moment it has the turn, before it is
    /// given up as hung: 20 s, plus a quarter of the audio's length (4x faster than real time).
    ///
    /// A working decode stays far inside it. The live windows decoded in 85 ms on average and at
    /// most 141 ms on the AMI replay, windows of up to 12 s of audio (about 85x real time), and
    /// FluidAudio's Parakeet was measured at about 110x real time on the Neural Engine when the
    /// engines were chosen, so 4x leaves room for a busy or throttled Mac. The 20 s floor covers
    /// the first decode after a load, when Core ML may still be preparing the model. Live windows
    /// hold at most 30 s of audio (`LiveWindowConfig.maxBuffer`), so a hung live decode is given up
    /// within 27.5 s; the offline fallback decodes a whole dictation take, or a region the core's
    /// final pass cut, and gets time in proportion (an hour of audio: 15 min 20 s).
    public static func decodeLimit(samples: Int) -> Duration {
        .seconds(20) + .seconds(Double(samples) / Double(sampleRate) / 4)
    }

    /// Loads the models, or waits for the load already under way. A failed load can be retried.
    public func load() async throws(ParakeetError) {
        _ = try await loaded()
    }

    private func loaded() async throws(ParakeetError) -> any ParakeetBackend {
        if let backend { return backend }
        let task: Task<Result<any ParakeetBackend, ParakeetError>, Never>
        if let loading {
            task = loading
        } else {
            let loader = self.loader
            task = Task {
                do throws(ParakeetError) { return .success(try await loader()) } catch { return .failure(error) }
            }
            loading = task
            loads += 1
        }
        let result = await task.value
        if loading == task { loading = nil }
        switch result {
        case .success(let b):
            backend = b
            return b
        case .failure(let e):
            throw e
        }
    }

    /// Decodes one window of the live stream.
    public func decode(_ samples: [Float]) async throws(ParakeetError) -> DecodedWindow {
        DecodedWindow(words: try await transcribe(samples).words)
    }

    /// Transcribes a buffer of any length: the offline path, as FluidAudio runs it (long audio in
    /// its own overlapping chunks), as the engine choice measured it.
    public func transcribe(_ samples: [Float]) async throws(ParakeetError) -> Transcribed {
        let backend = try await loaded()
        await turn.take()
        let limit = self.limit(samples.count)
        // Always returns, so the turn is always given back (see the file header).
        let outcome = await Self.bounded(limit) { () async -> Result<Transcribed, ParakeetError> in
            do throws(ParakeetError) {
                return .success(try await backend.transcribe(samples))
            } catch {
                return .failure(error)
            }
        }
        await turn.give()
        guard let outcome else {
            Log.engine.error("parakeet: a decode did not return within \(limit, privacy: .public); given up (it may still run on the Neural Engine)")
            throw .decodeTimedOut
        }
        return try outcome.get()
    }

    /// Runs `decode` against `limit`: its result, or nil once the limit passes first. A task group
    /// would not do: it waits for every child, and a decode that never returns would hold it too.
    private static func bounded(
        _ limit: Duration, _ decode: @escaping @Sendable () async -> Result<Transcribed, ParakeetError>
    ) async -> Result<Transcribed, ParakeetError>? {
        await withCheckedContinuation { (continuation: CheckedContinuation<Result<Transcribed, ParakeetError>?, Never>) in
            let race = Race(continuation)
            // Detached so neither runs on this actor; at the caller's priority (a live stream
            // decodes at user-initiated).
            let priority = Task.currentPriority
            race.enter(Task.detached(priority: priority) { race.settle(await decode()) })
            race.enter(Task.detached(priority: priority) {
                try? await Task.sleep(for: limit)
                race.settle(nil)
            })
        }
    }
}

/// A decode and its time limit, settled once by whichever ends first. The other is cancelled: a
/// timer stops sleeping, and a decode is asked to stop, which a Core ML prediction under way does
/// not notice.
private final class Race: Sendable {
    private struct State {
        var continuation: CheckedContinuation<Result<Transcribed, ParakeetError>?, Never>?
        var racers: [Task<Void, Never>] = []
    }

    private let state: Mutex<State>

    init(_ continuation: CheckedContinuation<Result<Transcribed, ParakeetError>?, Never>) {
        state = Mutex(State(continuation: continuation))
    }

    /// A racer, cancelled once the race is settled (at once if it already is).
    func enter(_ racer: Task<Void, Never>) {
        let settled = state.withLock { s in
            if s.continuation == nil { return true }
            s.racers.append(racer)
            return false
        }
        if settled { racer.cancel() }
    }

    /// Settles the race with `outcome` unless it is settled already.
    func settle(_ outcome: Result<Transcribed, ParakeetError>?) {
        let (continuation, racers) = state.withLock { s in
            defer { (s.continuation, s.racers) = (nil, []) }
            return (s.continuation, s.racers)
        }
        guard let continuation else { return }
        // The winner is among them; cancelling a task that is finishing does nothing.
        for racer in racers { racer.cancel() }
        continuation.resume(returning: outcome)
    }
}

/// A first-come, first-served turn for async callers.
actor Turn {
    private var taken = false
    private var waiting: [CheckedContinuation<Void, Never>] = []

    func take() async {
        if !taken {
            taken = true
            return
        }
        await withCheckedContinuation { waiting.append($0) }
    }

    func give() {
        if waiting.isEmpty {
            taken = false
        } else {
            waiting.removeFirst().resume()
        }
    }
}

/// Parakeet TDT v3 through FluidAudio 0.15.5.
struct FluidAudioParakeet: ParakeetBackend {
    let manager: AsrManager

    /// Loads v3 from FluidAudio's cache, refusing to download (see the file header).
    static func load() async throws(ParakeetError) -> any ParakeetBackend {
        #if !arch(arm64)
            throw ParakeetError.unsupported
        #else
            // Before any loader is touched (FluidAudio reads it per request).
            ModelHub.offlineMode = true
            let dir = AsrModels.defaultCacheDirectory(for: .v3)
            guard AsrModels.modelsExist(at: dir, version: .v3) else {
                throw ParakeetError.modelMissing
            }
            do {
                let models = try await AsrModels.load(from: dir, version: .v3)
                let manager = AsrManager(config: .default)
                try await manager.loadModels(models)
                return FluidAudioParakeet(manager: manager)
            } catch let error as DownloadError {
                if case .networkDisabled = error { throw ParakeetError.downloadRefused }
                throw ParakeetError.modelMissing
            } catch {
                throw ParakeetError.loadFailed(code: 1)
            }
        #endif
    }

    func transcribe(_ samples: [Float]) async throws(ParakeetError) -> Transcribed {
        do {
            // Fresh per call, as it was measured: the state carries the previous audio's context.
            var state = try TdtDecoderState()
            let result = try await manager.transcribe(samples, decoderState: &state, language: .english)
            let words = buildWordTimings(from: result.tokenTimings ?? []).map {
                TimedWord(
                    text: $0.word,
                    start: Int(($0.startTime * Double(sampleRate)).rounded()),
                    end: Int(($0.endTime * Double(sampleRate)).rounded()))
            }
            return Transcribed(text: result.text, words: words, reportedDuration: result.duration)
        } catch let error as ASRError {
            if case .invalidAudioData = error { throw ParakeetError.decodeFailed(code: 2) }
            throw ParakeetError.decodeFailed(code: 1)
        } catch {
            throw ParakeetError.decodeFailed(code: 1)
        }
    }
}
