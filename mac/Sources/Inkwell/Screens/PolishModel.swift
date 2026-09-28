// The Polish toggle (Settings > AI, and the first-run sheet): the user's switch, the consent that
// turns it on, and whether a language model can do it.
//
// Polish sends what the user dictates to a language model before it is typed, so it is off until
// the user turns it on through a consent step that says where the words go: Apple's on-device model
// (they stay on this Mac) or a named cloud provider (they leave it). Switching the toggle on only
// asks (`pendingConsent`); Allow sends `polish.allow` naming what was shown, and the core records
// the consent and turns polish on. Cancel sends nothing, so polish stays off. The core is the
// source of truth: `polish.state` says the switch, where polish would send now, and whether the
// consent covers that. When the model moves elsewhere (on-device to a cloud provider, or one
// provider to another) the consent no longer covers it: polish is paused, the toggle reads off, the
// line under it says why, and turning it on asks again. The core refuses to polish meanwhile, so
// nothing depends on this screen being open.
//
// The toggle reads "on" only when the switch is on, the consent covers the model, and the core has
// a working language model registered (engine.registered, kind llm, not let go of since). With no
// working model it reads off, cannot be switched, and says why, from what the Apple engines found.
//
// It also tells "polish keeps timing out" from an ordinary failure: a take whose polish ran out of
// its time budget arrives as the dictation warning polish_timed_out, while a polish cancelled for
// another reason (the core shutting down) stays polish_failed.
//
// A state that could not be read, a consent that could not be recorded and a switch that could not
// be saved each say so under the toggle ("couldn't …"), never read as off.
import AppleEngines
import InkBridge
import Observation

@MainActor
@Observable
final class PolishModel {
    /// Where polish sends a dictation, as the consent step names it.
    struct Destination: Equatable, Sendable {
        enum Kind: Equatable, Sendable {
            case onDevice
            case cloud(endpoint: String)
        }

        let kind: Kind
        /// The model's name as the core reports it (for a cloud model, its provider's name).
        let name: String

        var isOnDevice: Bool { kind == .onDevice }

        /// The words for it: "Apple's on-device model", or the provider's name.
        var label: String {
            switch kind {
            case .onDevice:
                name == FoundationModelsPolish.modelName ? "Apple's on-device model" : "\(name), on this Mac"
            case .cloud:
                name.isEmpty ? "a cloud provider" : name
            }
        }
    }

    /// The core's polish state, as this model keeps it.
    struct Snapshot: Equatable, Sendable {
        /// The user's switch.
        var on: Bool
        /// Whether the consent covers the model polish would use now.
        var allowed: Bool
        /// Where polish would send now; nil with no model.
        var destination: Destination?
        /// What the core could not read ("couldn't read …").
        var error: String?

        init(on: Bool, allowed: Bool, destination: Destination?, error: String?) {
            self.on = on
            self.allowed = allowed
            self.destination = destination
            self.error = error
        }

        init(_ state: PolishState) {
            let destination: Destination? = switch state.to {
            case .onDevice: Destination(kind: .onDevice, name: state.name ?? "")
            case .cloud: state.endpoint.map { Destination(kind: .cloud(endpoint: $0), name: state.name ?? "") }
            case nil: nil
            }
            self.init(on: state.on, allowed: state.allowed, destination: destination, error: state.error)
        }
    }

    /// What the Apple engines found for polish, once they registered (nil before).
    private(set) var appleState: AppleEngineState?
    /// The core's polish state (nil until read).
    private(set) var state: Snapshot?
    /// Language models the core confirmed and still holds, by id.
    private(set) var models: Set<String> = []
    /// Polish timeouts in a row, across takes; a take that polished resets it.
    private(set) var timeoutsInARow = 0
    /// Something could not be read, recorded or saved.
    private(set) var failure: Failure?
    /// The consent step on screen: where polish would send, waiting for Allow or Cancel.
    private(set) var pendingConsent: Destination?
    /// Which screen asked, so only that one shows the step (the first-run sheet can sit over
    /// Settings).
    private(set) var consentHost: ConsentHost?

    /// The screens that can turn polish on.
    enum ConsentHost: Equatable, Sendable {
        case settings
        case onboarding
    }

    enum Failure: Equatable, Sendable {
        case read
        case write
        /// The consent could not be recorded (the model changed while the user read, or the store
        /// refused): polish stays as it was.
        case allow
    }

    /// Timeouts in a row at which the toggle warns.
    static let timeoutWarning = 2

    /// The ids of this model's commands (a `command.failed` carries them).
    static let settingID = "setting:\(ShellSetting.dictationPolish.rawValue)"
    static let getID = "polish.get"
    static let allowID = "polish.allow"

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var takeTimedOut = false
    /// The state before a switch-off the core has not confirmed, to put back if saving fails.
    @ObservationIgnored private var beforeOff: Snapshot??

    init(send: @escaping SendCommand) {
        self.send = send
    }

    /// Whether a language model can polish now.
    var hasWorkingEngine: Bool { !models.isEmpty }

    /// The user's switch, from the core (nil until read).
    var preference: Bool? { state?.on }

    /// Where polish would send now, if the core named a model.
    var destination: Destination? { state?.destination }

    /// What the toggle shows: on only with the switch, a consent that covers the model, and a
    /// working engine.
    var isOn: Bool { state?.on == true && state?.allowed == true && hasWorkingEngine }

    /// On, but the model now sends somewhere the user has not agreed to: nothing is polished.
    var isPaused: Bool { state?.on == true && state?.allowed == false && hasWorkingEngine && destination != nil }

    /// Whether the toggle can be switched.
    var canToggle: Bool { hasWorkingEngine && state != nil && destination != nil }

    /// Polish has timed out on several takes in a row.
    var keepsTimingOut: Bool { timeoutsInARow >= Self.timeoutWarning }

    /// The line under the toggle.
    var status: String {
        switch failure {
        case .read: return "Couldn't read your polish setting. Open Settings again to retry."
        case .write: return "Couldn't save the change, so polish stays as it was."
        case .allow: return "Couldn't turn polish on, so it stays off. Try again."
        case nil: break
        }
        guard hasWorkingEngine else { return Self.unavailable(appleState) }
        if let error = state?.error, state?.on == true {
            // The consent could not be read: the core treats that as none.
            return "\(Self.sentence(error)), so polish is paused. Turn it on again to allow it."
        }
        if isPaused, let destination {
            return destination.isOnDevice
                ? "Paused: polish now uses \(destination.label). Turn it on again to allow it."
                : "Paused: polish would now send your words to \(destination.label). Turn it on again to allow it."
        }
        if keepsTimingOut {
            return "Polish keeps timing out, so your words go in as you said them."
        }
        guard isOn, let destination else { return "Off. Your words go in as you said them." }
        return destination.isOnDevice
            ? "On, with \(destination.label). Your words stay on this Mac."
            : "On. Your words go to \(destination.label) before they are typed."
    }

    /// Why nothing can polish, in the user's terms.
    static func unavailable(_ state: AppleEngineState?) -> String {
        switch state {
        case nil:
            return "Checking whether Apple Intelligence can polish on this Mac…"
        case .registered:
            // Registered by the engines, not yet confirmed by the core.
            return "Starting…"
        case .unavailable(let code):
            switch AppleIntelligence.Reason(rawValue: code) {
            case .deviceNotEligible: return "Polish needs Apple Intelligence, which this Mac can't run."
            case .notEnabled: return "Polish needs Apple Intelligence. Turn it on in System Settings."
            case .modelNotReady: return "Apple Intelligence is still getting ready. Polish turns on when it is."
            case .unsupported: return "Polish needs a version of macOS with Apple Intelligence."
            case .other, nil: return "Apple Intelligence isn't available right now."
            }
        case .modelMissing:
            return "Polish's model is not on this Mac."
        case .failed(let code):
            return "Polish could not start (error \(code))."
        }
    }

    // MARK: - The consent step's words

    static let consentTitle = "Turn on polish?"

    /// What the consent step says: what polish does, and where the words go for this model.
    static func consentMessage(_ destination: Destination) -> String {
        let what = "Polish sends what you dictate to a language model, which tidies the wording before it is typed."
        return destination.isOnDevice
            ? "\(what) It uses \(destination.label), so your words stay on this Mac."
            : "\(what) It uses \(destination.label), a cloud provider: your words leave this Mac and go to \(destination.label)."
    }

    /// The button that agrees.
    static func consentButton(_ destination: Destination) -> String {
        destination.isOnDevice ? "Turn On Polish" : "Send to \(destination.label)"
    }

    // MARK: - Commands

    /// Reads the state from the core.
    func load() {
        send(.polishGet)
    }

    /// The Apple engines registered polish, or found it could not run.
    func appleEnginesReported(_ state: AppleEngineState) {
        appleState = state
    }

    /// The user switched the toggle. On shows the consent step (nothing is sent until Allow); off
    /// turns polish off, which also withdraws the consent. Does nothing without a working engine.
    func setOn(_ on: Bool, from host: ConsentHost = .settings) {
        guard canToggle else { return }
        failure = nil
        if on {
            pendingConsent = destination
            consentHost = host
            return
        }
        pendingConsent = nil
        consentHost = nil
        if beforeOff == nil {
            beforeOff = .some(state)
        }
        state?.on = false
        state?.allowed = false
        send(.settingSet(.dictationPolish, "off"))
    }

    /// The consent step's Allow: the user agreed to where polish sends. The core records it only if
    /// that is still where the model goes.
    func allowConsent() {
        guard let destination = pendingConsent else { return }
        pendingConsent = nil
        consentHost = nil
        failure = nil
        switch destination.kind {
        case .onDevice: send(.polishAllow(to: .onDevice, endpoint: nil))
        case .cloud(let endpoint): send(.polishAllow(to: .cloud, endpoint: endpoint))
        }
    }

    /// The consent step's Cancel (or the step dismissed): nothing is sent, polish stays as it was.
    func cancelConsent() {
        pendingConsent = nil
        consentHost = nil
    }

    /// Whether the status is a problem to show in the alert colour.
    var isProblem: Bool { failure != nil || keepsTimingOut || isPaused || (state?.error != nil && state?.on == true) }

    func apply(_ event: InkEvent) {
        switch event {
        case .engineRegistered(let engine) where engine.kind == .llm:
            models.insert(engine.id)
            // Where polish sends may have changed.
            load()
        case .engineUnregistered(let engine):
            if models.remove(engine.id) != nil {
                load()
            }
        case .coreStopped:
            models = []
            cancelConsent()
        case .polishState(let value):
            state = Snapshot(value)
            beforeOff = nil
            if failure == .read || failure == .write { failure = nil }
            // The step names a destination that is no longer the model's: close it rather than
            // change its words under the user's finger. The line under the toggle says where polish
            // goes now, and switching it on asks about that.
            if let pending = pendingConsent, pending != destination {
                cancelConsent()
            }
        case .commandFailed(let failed) where failed.id == Self.settingID:
            if failed.command == "setting.set" {
                failure = .write
                if let before = beforeOff {
                    state = before
                }
                beforeOff = nil
            } else {
                failure = .read
            }
        case .commandFailed(let failed) where failed.id == Self.getID:
            failure = .read
        case .commandFailed(let failed) where failed.id == Self.allowID:
            failure = .allow
        case .dictationStarted:
            takeTimedOut = false
        case .dictationWarningEvent(let warning) where warning.kind == .polishTimedOut:
            takeTimedOut = true
            timeoutsInARow += 1
        case .dictationWarningEvent(let warning) where warning.kind == .polishNotAllowed:
            // The core refused to send: read where polish goes now, so the toggle says why.
            load()
        case .dictationInserted:
            // A take that went out without timing out, while polish was on, ends the run.
            if !takeTimedOut && isOn {
                timeoutsInARow = 0
            }
        default:
            break
        }
    }

    /// "couldn't read x" as the start of a sentence.
    private static func sentence(_ s: String) -> String {
        s.prefix(1).uppercased() + s.dropFirst()
    }
}
