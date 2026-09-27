// Parakeet as an offline engine: a whole buffer, FluidAudio's own chunking for long audio, the
// configuration the engine choice was measured with.
//
// `AppleEngines` does not register it: architecture rule 10 gives both finals to Qwen3-ASR. It
// exists so the AMI harness can run the measured configuration through the core's offline path
// (`ink_register_engine`, a worker thread, `ink_engine_complete`), and as the finals' fallback
// before Qwen3-ASR is installed, should that be wanted.

import Foundation
import InkBridge

/// Parakeet TDT v3, transcribing whole buffers for the core.
public final class ParakeetOfflineEngine: InkOfflineEngine {
    public let id: String
    public let licence = "CC-BY-4.0"
    public let jobs: [(job: Job, wer: Double)]
    private let model: ParakeetModel

    /// `jobs` with their measured rates; by default the meeting final at the AMI IHM rate (23.4).
    public init(
        model: ParakeetModel = .shared,
        id: String = "fluidaudio-parakeet-tdt-0.6b-v3-offline",
        jobs: [(job: Job, wer: Double)] = [(.meetingFinal, 23.4)]
    ) {
        self.model = model
        self.id = id
        self.jobs = jobs
    }

    public func transcribe(
        _ samples: [Float], channel: Channel, context: String?,
        completion: @escaping @Sendable (Result<[InkSegment], InkEngineError>) -> Void
    ) {
        // Shorter than FluidAudio takes: nothing it could hear.
        guard samples.count >= LiveWindowConfig().minimum else { return completion(.success([])) }
        let model = self.model
        Task {
            do throws(ParakeetError) {
                let result = try await model.transcribe(samples)
                // Timed from the sample count, never FluidAudio's duration (0 past 15 s).
                let endMs = UInt64(samples.count / (sampleRate / 1_000))
                let text = result.text.trimmingCharacters(in: .whitespacesAndNewlines)
                completion(.success(text.isEmpty ? [] : [InkSegment(startMs: 0, endMs: endMs, text: text)]))
            } catch {
                completion(.failure(error.engineError))
            }
        }
    }
}
