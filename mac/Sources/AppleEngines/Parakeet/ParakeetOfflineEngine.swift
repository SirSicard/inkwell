// Parakeet as an offline engine: a whole buffer, FluidAudio's own chunking for long audio, the
// configuration its word error rates were measured with.
//
// It is the finals' fallback. Architecture rule 10 gives the dictation and meeting finals to
// Qwen3-ASR; `AppleEngines` registers this engine for both with the rates measured for Parakeet,
// which are worse on both, so the core's router, which picks the lowest rate among installed
// models and registered engines at every call, uses it only while Qwen3-ASR is not installed (its
// 2.3 GB download not finished, or failed) and hands the jobs back once it is. The shell sees which
// serves through `engine.route` (inkwell.h).

import Foundation
import InkBridge

/// Parakeet TDT v3, transcribing whole buffers for the core.
public final class ParakeetOfflineEngine: InkOfflineEngine {
    public let id: String
    public let licence = "CC-BY-4.0"
    public let jobs: [(job: Job, wer: Double)]
    private let model: ParakeetModel

    /// The id the fallback registers under.
    public static let fallbackID = "fluidaudio-parakeet-tdt-0.6b-v3-offline"

    /// The rates measured for Parakeet TDT v3 on FluidAudio when the engines were chosen:
    /// FLEURS en for dictation (6.7; Qwen3-ASR 4.59) and AMI IHM for meetings (23.4; Qwen3-ASR
    /// 16.08). Being worse on both is what keeps it the fallback: never raise it above the
    /// registry's model without new measurements.
    public static let measured: [(job: Job, wer: Double)] = [(.dictationFinal, 6.7), (.meetingFinal, 23.4)]

    /// The finals' fallback: both jobs at their measured rates.
    public static func fallback(model: ParakeetModel = .shared) -> ParakeetOfflineEngine {
        ParakeetOfflineEngine(model: model, id: fallbackID, jobs: measured)
    }

    /// `jobs` with their measured rates; by default the meeting final alone.
    public init(
        model: ParakeetModel = .shared,
        id: String = ParakeetOfflineEngine.fallbackID,
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
