// Apple accelerators, registered into the core as engines over the C ABI (architecture rule 2):
// FluidAudio's Parakeet for live partials, and for the finals until Qwen3-ASR is installed, and
// Foundation Models for polish.
//
// Each is registered only when it can run, and the core's router then picks it for its job: live
// partials go to the registered streaming engine, and polish to the registered language model.
// What each came to is returned as a state for the app to show; nothing is faked when an engine
// cannot run.

import InkBridge
import Synchronization

/// What an Apple engine came to on this Mac.
public enum AppleEngineState: Sendable, Equatable {
    /// Registered: the core uses it.
    case registered
    /// Its model files are not on this Mac. Nothing was downloaded.
    case modelMissing
    /// It cannot run here now (the code says why: `AppleIntelligence.Reason` for polish, 1 for a
    /// Mac without Apple Silicon for live partials).
    case unavailable(code: Int)
    /// It failed to load or register (the code is the engine's or the core's).
    case failed(code: Int)
}

/// The engines' states after `AppleEngines.register()`.
public struct AppleEnginesReport: Sendable, Equatable {
    /// Parakeet, streaming.
    public var livePartials: AppleEngineState
    /// Parakeet, as the dictation and meeting finals' fallback (`ParakeetOfflineEngine`). Whether
    /// it serves now is the router's choice: ask with `engine.route`.
    public var finals: AppleEngineState
    /// Foundation Models.
    public var polish: AppleEngineState
}

/// The Apple engines of one session. The shell keeps one next to its `InkSession`.
public final class AppleEngines: Sendable {
    private let session: InkSession
    private let parakeet: ParakeetModel
    /// The polish model this registers while Apple Intelligence is available.
    private let polishModel: FoundationModelsPolish
    /// Whether `polishModel` is registered now: what `syncPolish` compares against.
    private let polish = Mutex<FoundationModelsPolish?>(nil)
    /// Parakeet's registration in this session (`registerParakeet`).
    private let parakeetRegistration = Mutex(ParakeetRegistration())

    /// Whether `register` has run (Parakeet is then this session's to register, also once its
    /// download finishes), and what registering its engines came to: done at most once.
    private struct ParakeetRegistration {
        var asked = false
        var outcome: (live: AppleEngineState, finals: AppleEngineState)?
    }

    public init(
        session: InkSession, parakeet: ParakeetModel = .shared,
        polish: FoundationModelsPolish = FoundationModelsPolish()
    ) {
        self.session = session
        self.parakeet = parakeet
        self.polishModel = polish
    }

    /// Forward the core's events here (the shell's event handler, on the core's event thread; it
    /// returns at once). A take starting prewarms polish, so the polish at its release does not
    /// pay the model's cold start. Parakeet's download finishing loads it and registers its
    /// engines then, not at the next launch (once `register` has run, and only if they are not
    /// registered yet).
    public func handle(_ event: InkEvent) {
        switch event {
        case .dictationStarted:
            prewarmPolish()
        case .modelUpdateFinished(let finished) where finished.ok && finished.next == ParakeetFiles.rowID:
            guard parakeetRegistration.withLock({ $0.asked && $0.outcome == nil }) else { return }
            Task {
                let (live, finals) = await registerParakeet()
                Log.engine.notice("parakeet after its download: live \(String(describing: live), privacy: .public), finals \(String(describing: finals), privacy: .public)")
            }
        default:
            break
        }
    }

    /// Prewarms polish if it is registered (so never while Apple Intelligence is unavailable).
    public func prewarmPolish() {
        polish.withLock { $0 }?.prewarm()
    }

    /// Loads Parakeet (once per process; seconds on the first launch) and registers it for live
    /// partials and as the finals' fallback, and registers polish if Apple Intelligence is
    /// available. Call once the session has started; call `syncPolish` again whenever Apple
    /// Intelligence may have changed. Parakeet not downloaded yet is registered when its download
    /// finishes (`handle`).
    public func register() async -> AppleEnginesReport {
        parakeetRegistration.withLock { $0.asked = true }
        let (live, finals) = await registerParakeet()
        return AppleEnginesReport(livePartials: live, finals: finals, polish: syncPolish())
    }

    /// Loads Parakeet and registers its engines, unless they are registered already: whoever
    /// loaded it (the launch, or its download finishing) registers them once.
    private func registerParakeet() async -> (live: AppleEngineState, finals: AppleEngineState) {
        do throws(ParakeetError) {
            try await parakeet.load()
        } catch .modelMissing {
            // Expected until the user agrees to the download: nothing is fetched here.
            Log.engine.notice("parakeet: its models are not on this Mac; no live partials or finals fallback")
            return (.modelMissing, .modelMissing)
        } catch .downloadRefused {
            Log.engine.notice("parakeet: a model file was missing and offline mode refused to fetch it; no live partials or finals fallback")
            return (.modelMissing, .modelMissing)
        } catch .unsupported {
            Log.engine.notice("parakeet: this Mac has no Apple Silicon; no live partials or finals fallback")
            return (.unavailable(code: 1), .unavailable(code: 1))
        } catch {
            let code = error.engineError.code
            Log.engine.error("parakeet: loading failed (code \(code)); no live partials or finals fallback")
            return (.failed(code: code), .failed(code: code))
        }
        let (session, parakeet) = (self.session, self.parakeet)
        return parakeetRegistration.withLock { registration in
            if let outcome = registration.outcome { return outcome }
            let live = Self.state { try session.register(ParakeetLiveEngine(decoder: parakeet)) }
            let finals = Self.state { try session.register(ParakeetOfflineEngine.fallback(model: parakeet)) }
            registration.outcome = (live, finals)
            return (live, finals)
        }
    }

    /// Registers polish when Apple Intelligence is available and it is not registered, and lets go
    /// of it when Apple Intelligence is not. Returns its state now.
    @discardableResult
    public func syncPolish(availability: AppleIntelligence = AppleIntelligence.current()) -> AppleEngineState {
        polish.withLock { registered in
            switch availability {
            case .available:
                if registered != nil { return .registered }
                let model = polishModel
                let result = Self.state { try session.register(model) }
                if result == .registered { registered = model }
                return result
            case .unavailable(let reason):
                if let model = registered {
                    // The core may still hold it for a call in flight; that call gets
                    // "unavailable" from the model itself.
                    do {
                        try session.unregister(id: model.id)
                    } catch {
                        // Only a core that has stopped refuses the command, and it holds no
                        // engines any more.
                        Log.engine.notice("polish could not be let go of: the core has stopped")
                    }
                    registered = nil
                }
                return .unavailable(code: reason.rawValue)
            }
        }
    }

    private static func state(_ register: () throws -> Void) -> AppleEngineState {
        do {
            try register()
            return .registered
        } catch let error as InkStatusError {
            Log.engine.error("an Apple engine was refused by the core (status \(error.code))")
            return .failed(code: Int(error.code))
        } catch {
            return .failed(code: 1)
        }
    }
}

extension InkEngineError {
    /// The error's code, for a state.
    var code: Int {
        switch self {
        case .failed(let code), .unavailable(let code): code
        case .cancelled: 2
        case .modelMissing: 3
        case .badRequest: 4
        }
    }
}
