// Dictation polish on Apple's on-device model (Foundation Models), registered with the core as a
// language model.
//
// Apple Intelligence may be off, still downloading its model, or not supported on this Mac (Intel
// Macs, among others). That is a state the app shows, never something papered over: while it is
// unavailable the engine is not registered (`AppleEngines.syncPolish()`), and a call that finds it
// unavailable anyway answers `.unavailable`, so the dictation goes out as written. No answer is
// ever made up.
//
// Prewarm: the model's first answer after a while idle pays a cold start (1.4 s measured). A take
// that will be polished is known when it starts, so `AppleEngines` prewarms on `dictation.started`
// (the key held past the minimum hold, 200 ms after key-down): a session with the instructions the
// last polish used is prepared and loaded while the user speaks, and the polish at the release
// answers on it. Never while Apple Intelligence is unavailable.

import Foundation
import InkBridge
import Synchronization
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

/// What runs the requests: the system model, or a test's fake.
public protocol PolishBackend: Sendable {
    /// Prepares a session with `instructions` and loads the model, without waiting for it.
    func prewarm(instructions: String?)
    /// One answer. Uses the prepared session when its instructions are the request's.
    func respond(_ request: InkLlmRequest) async throws -> String
}

/// Polish on the on-device model.
public final class FoundationModelsPolish: InkLanguageModel {
    /// Answers a request on the model (a test's fake).
    public typealias Respond = @Sendable (InkLlmRequest) async throws -> String

    public let id = "apple-foundation-models"
    /// Apple's system model, used through the OS under its terms; nothing is redistributed.
    public let licence = "Apple system model"
    public let model = "SystemLanguageModel.default"
    /// It runs on this Mac: nothing leaves it.
    public let isLocal = true

    private let availability: @Sendable () -> AppleIntelligence
    private let backend: any PolishBackend
    /// The instructions of the last request: what a prewarm prepares a session for.
    private let lastInstructions = Mutex<String?>(nil)

    public init(
        availability: @escaping @Sendable () -> AppleIntelligence = AppleIntelligence.current,
        backend: any PolishBackend = SystemModel()
    ) {
        self.availability = availability
        self.backend = backend
    }

    /// A model answering with `respond` (tests).
    public convenience init(
        availability: @escaping @Sendable () -> AppleIntelligence = AppleIntelligence.current,
        respond: @escaping Respond
    ) {
        self.init(availability: availability, backend: Answering(respond: respond))
    }

    /// Loads the model and prepares a session for the next polish, without waiting. Does nothing
    /// while Apple Intelligence is unavailable.
    public func prewarm() {
        guard case .available = availability() else { return }
        backend.prewarm(instructions: lastInstructions.withLock { $0 })
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
        lastInstructions.withLock { $0 = request.system }
        let backend = self.backend
        let task = Task {
            do {
                let text = try await backend.respond(request)
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

    /// The on-device model: a fresh session per request (polish keeps no history), with the
    /// request's system prompt as its instructions, or the one a prewarm prepared for them.
    public final class SystemModel: PolishBackend {
        #if canImport(FoundationModels)
            /// The session a prewarm prepared, with its instructions; used once.
            private let prepared = Mutex<(instructions: String?, session: LanguageModelSession)?>(nil)
        #endif

        public init() {}

        public func prewarm(instructions: String?) {
            #if canImport(FoundationModels)
                let session = LanguageModelSession(model: .default, instructions: instructions)
                // Returns at once; the model loads in the background.
                session.prewarm()
                prepared.withLock { $0 = (instructions, session) }
            #endif
        }

        public func respond(_ request: InkLlmRequest) async throws -> String {
            #if canImport(FoundationModels)
                let session = prepared.withLock { p -> LanguageModelSession? in
                    defer { p = nil }
                    return p?.instructions == request.system ? p?.session : nil
                } ?? LanguageModelSession(model: .default, instructions: request.system)
                let options = GenerationOptions(
                    temperature: request.temperature, maximumResponseTokens: request.maxTokens)
                return try await session.respond(to: request.user, options: options).content
            #else
                throw InkEngineError.unavailable(code: AppleIntelligence.Reason.unsupported.rawValue)
            #endif
        }
    }

    /// A backend answering with a closure, with nothing to prewarm.
    struct Answering: PolishBackend {
        let respond: Respond
        func prewarm(instructions: String?) {}
        func respond(_ request: InkLlmRequest) async throws -> String { try await respond(request) }
    }

    /// The model's errors as codes: never its text, which can quote the request. (Every payload
    /// carries a debug description, and a parsing error the raw answer; none is read.)
    ///
    /// macOS 27 throws new types in place of the deprecated `GenerationError`. Measured on macOS
    /// 27.2: a prompt past the context window threw `LanguageModelError.contextSizeExceeded`, and
    /// two requests at once on one session `LanguageModelSession.Error`; neither matches
    /// `GenerationError`, so both went out as code 1. Each type is mapped, to the same codes.
    static func engineError(_ error: any Error) -> InkEngineError {
        #if canImport(FoundationModels, _version: 2.0)
            if #available(macOS 27.0, *), let code = macOS27Error(error) { return code }
        #endif
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
                case .unsupportedGuide: return .failed(code: 17)
                // A case a later SDK adds; the compiler names any known case left out.
                @unknown default: return .failed(code: 19)
                }
            }
        #endif
        return .failed(code: 1)
    }

    // Compiled only against an SDK that declares the macOS 27 types: its FoundationModels is
    // module version 2 (the 26.x SDKs ship 1.5). CI builds with a 26 SDK, where `#available` alone
    // could not hide a type the SDK lacks. The module's version is checked rather than the
    // compiler's, since it is the SDK that has the types or not; both were verified to skip the
    // block with Swift 6.3 and the 26.5 SDK, and to compile it with Swift 6.4 and the 27 SDK.
    #if canImport(FoundationModels, _version: 2.0)
        /// A macOS 27 error as a code, or nil for any other error.
        @available(macOS 27.0, *)
        static func macOS27Error(_ error: any Error) -> InkEngineError? {
            switch error {
            case let error as LanguageModelError:
                switch error {
                case .contextSizeExceeded: return .failed(code: 10)
                case .guardrailViolation: return .failed(code: 11)
                case .refusal: return .failed(code: 12)
                case .unsupportedLanguageOrLocale: return .failed(code: 13)
                case .rateLimited: return .failed(code: 14)
                case .unsupportedGenerationGuide: return .failed(code: 17)
                // Failures `GenerationError` had no case for.
                case .timeout: return .failed(code: 20)
                case .unsupportedCapability: return .failed(code: 21)
                case .unsupportedTranscriptContent: return .failed(code: 22)
                @unknown default: return .failed(code: 19)
                }
            case let error as SystemLanguageModel.Error:
                switch error {
                case .assetsUnavailable:
                    return .unavailable(code: AppleIntelligence.Reason.modelNotReady.rawValue)
                @unknown default: return .failed(code: 19)
                }
            case let error as LanguageModelSession.Error:
                switch error {
                case .concurrentRequests: return .failed(code: 15)
                case .transcriptMutationWhileResponding: return .failed(code: 23)
                @unknown default: return .failed(code: 19)
                }
            case is GeneratedContent.ParsingError:
                return .failed(code: 16)
            default:
                return nil
            }
        }
    #endif
}
