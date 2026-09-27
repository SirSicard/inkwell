// Dictation polish on Apple's on-device model (Foundation Models), registered with the core as a
// language model.
//
// Apple Intelligence may be off, still downloading its model, or not supported on this Mac (Intel
// Macs, among others). That is a state the app shows, never something papered over: while it is
// unavailable the engine is not registered (`AppleEngines.syncPolish()`), and a call that finds it
// unavailable anyway answers `.unavailable`, so the dictation goes out as written. No answer is
// ever made up.

import Foundation
import InkBridge
#if canImport(FoundationModels)
    import FoundationModels
#endif

/// Whether Apple Intelligence can run the on-device model now.
public enum AppleIntelligence: Sendable, Equatable {
    case available
    case unavailable(Reason)

    /// Why not. The raw value is the code the core shows in its error.
    public enum Reason: Int, Sendable, Equatable {
        /// This Mac cannot run it (Intel Macs, among others).
        case deviceNotEligible = 1
        /// Apple Intelligence is turned off in System Settings.
        case notEnabled = 2
        /// The model is not ready yet (downloading, or the system is preparing it).
        case modelNotReady = 3
        /// This build or OS has no Foundation Models.
        case unsupported = 4
        /// A reason this build does not know.
        case other = 5
    }

    /// The system's answer now.
    public static func current() -> AppleIntelligence {
        #if canImport(FoundationModels)
            switch SystemLanguageModel.default.availability {
            case .available:
                return .available
            case .unavailable(.deviceNotEligible):
                return .unavailable(.deviceNotEligible)
            case .unavailable(.appleIntelligenceNotEnabled):
                return .unavailable(.notEnabled)
            case .unavailable(.modelNotReady):
                return .unavailable(.modelNotReady)
            case .unavailable:
                return .unavailable(.other)
            }
        #else
            return .unavailable(.unsupported)
        #endif
    }
}

/// Polish on the on-device model.
public final class FoundationModelsPolish: InkLanguageModel {
    /// Answers a request on the model; `FoundationModelsPolish.respond` in the app.
    public typealias Respond = @Sendable (InkLlmRequest) async throws -> String

    public let id = "apple-foundation-models"
    /// Apple's system model, used through the OS under its terms; nothing is redistributed.
    public let licence = "Apple system model"
    public let model = "SystemLanguageModel.default"
    /// It runs on this Mac: nothing leaves it.
    public let isLocal = true

    private let availability: @Sendable () -> AppleIntelligence
    private let respond: Respond

    public init(
        availability: @escaping @Sendable () -> AppleIntelligence = AppleIntelligence.current,
        respond: @escaping Respond = FoundationModelsPolish.respond
    ) {
        self.availability = availability
        self.respond = respond
    }

    public func generate(
        _ request: InkLlmRequest,
        cancellation: InkCancellation,
        completion: @escaping @Sendable (Result<String, InkEngineError>) -> Void
    ) {
        // Checked at every call: Apple Intelligence can be turned off while the engine is
        // registered.
        if case .unavailable(let reason) = availability() {
            return completion(.failure(.unavailable(code: reason.rawValue)))
        }
        // Polish asks for plain text. A structured request would need a generation schema,
        // which this engine does not build: refused, never answered loosely.
        guard request.jsonSchema == nil else {
            return completion(.failure(.badRequest))
        }
        let respond = self.respond
        let task = Task {
            do {
                let text = try await respond(request)
                completion(.success(text))
            } catch is CancellationError {
                completion(.failure(.cancelled))
            } catch let error as InkEngineError {
                completion(.failure(error))
            } catch {
                completion(.failure(Self.engineError(error)))
            }
        }
        cancellation.onCancel { task.cancel() }
    }

    /// One answer from the on-device model: a fresh session per request (polish keeps no
    /// history), the request's system prompt as its instructions.
    public static let respond: Respond = { request in
        #if canImport(FoundationModels)
            let session = LanguageModelSession(model: .default, instructions: request.system)
            let options = GenerationOptions(
                temperature: request.temperature, maximumResponseTokens: request.maxTokens)
            return try await session.respond(to: request.user, options: options).content
        #else
            throw InkEngineError.unavailable(code: AppleIntelligence.Reason.unsupported.rawValue)
        #endif
    }

    /// The model's errors as codes: never its text, which can quote the request.
    static func engineError(_ error: any Error) -> InkEngineError {
        #if canImport(FoundationModels)
            if let error = error as? LanguageModelSession.GenerationError {
                switch error {
                case .assetsUnavailable:
                    return .unavailable(code: AppleIntelligence.Reason.modelNotReady.rawValue)
                case .exceededContextWindowSize: return .failed(code: 10)
                case .guardrailViolation: return .failed(code: 11)
                case .refusal: return .failed(code: 12)
                case .unsupportedLanguageOrLocale: return .failed(code: 13)
                case .rateLimited: return .failed(code: 14)
                case .concurrentRequests: return .failed(code: 15)
                case .decodingFailure: return .failed(code: 16)
                default: return .failed(code: 19)
                }
            }
        #endif
        return .failed(code: 1)
    }
}
