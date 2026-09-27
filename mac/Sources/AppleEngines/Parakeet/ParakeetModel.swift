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

import FluidAudio
import Foundation

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

    public init(loader: @escaping Loader) {
        self.loader = loader
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
        do throws(ParakeetError) {
            let result = try await backend.transcribe(samples)
            await turn.give()
            return result
        } catch {
            await turn.give()
            throw error
        }
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
