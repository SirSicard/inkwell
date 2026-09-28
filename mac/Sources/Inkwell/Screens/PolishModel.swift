// The Polish toggle (Settings > AI, and the first-run sheet): the user's switch, the consent that
// turns it on (ConsentModel, feature polish), and whether a language model can do it.
//
// Polish sends what the user dictates to a language model before it is typed, so it is off until
// the user turns it on through a consent step that says where the words go (ConsentModel has the
// flow). Switching the toggle on only asks (`pendingConsent`); Allow sends `consent.allow`, Cancel
// sends nothing. Off sends `setting.set dictation.polish off`, which withdraws the consent in the
// same write.
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
    typealias Destination = ConsentModel.Destination
    typealias ConsentHost = ConsentModel.Host

    /// Polish's consent and switch, from the core.
    let consent: ConsentModel
    /// What the Apple engines found for polish, once they registered (nil before).
    private(set) var appleState: AppleEngineState?
    /// Language models the core confirmed and still holds, by id.
    private(set) var models: Set<String> = []
    /// Polish timeouts in a row, across takes; a take that polished resets it.
    private(set) var timeoutsInARow = 0

    /// Timeouts in a row at which the toggle warns.
    static let timeoutWarning = 2

    /// The id of this model's switch command (a `command.failed` carries it).
    static let settingID = "setting:\(ShellSetting.dictationPolish.rawValue)"

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var takeTimedOut = false

    init(send: @escaping SendCommand) {
        self.send = send
        consent = ConsentModel(feature: .polish, switchSettingID: Self.settingID, send: send)
    }

    /// Whether a language model can polish now.
    var hasWorkingEngine: Bool { !models.isEmpty }

    /// The core's state (nil until read).
    var state: ConsentModel.Snapshot? { consent.state }
    /// The user's switch, from the core (nil until read).
    var preference: Bool? { consent.state?.on }
    /// Where polish would send now, if the core named a model.
    var destination: Destination? { consent.destination }
    /// Something could not be read, recorded or saved.
    var failure: ConsentModel.Failure? { consent.failure }
    /// The consent step on screen.
    var pendingConsent: Destination? { consent.pending }
    /// Which screen asked for the consent step.
    var consentHost: ConsentHost? { consent.host }

    /// What the toggle shows: on only with the switch, a consent that covers the model, and a
    /// working engine.
    var isOn: Bool { consent.isAllowedOn && hasWorkingEngine }

    /// On, but the model now sends somewhere the user has not agreed to: nothing is polished.
    var isPaused: Bool { consent.isPaused && hasWorkingEngine }

    /// Whether the toggle can be switched.
    var canToggle: Bool { hasWorkingEngine && consent.state != nil && destination != nil }

    /// Polish has timed out on several takes in a row.
    var keepsTimingOut: Bool { timeoutsInARow >= Self.timeoutWarning }

    /// The line under the toggle.
    var status: String {
        if consent.failure != nil, let problem = consent.problem { return problem }
        guard hasWorkingEngine else { return Self.unavailable(appleState) }
        if let problem = consent.problem { return problem }
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

    static let consentTitle = ConsentModel.title(.polish)

    static func consentMessage(_ destination: Destination) -> String {
        ConsentModel.message(.polish, destination)
    }

    static func consentButton(_ destination: Destination) -> String {
        ConsentModel.button(.polish, destination)
    }

    // MARK: - Commands

    /// Reads the state from the core.
    func load() {
        consent.load()
    }

    /// The Apple engines registered polish, or found it could not run.
    func appleEnginesReported(_ state: AppleEngineState) {
        appleState = state
    }

    /// The user switched the toggle. On shows the consent step (nothing is sent until Allow); off
    /// turns polish off, which also withdraws the consent. Does nothing without a working engine.
    func setOn(_ on: Bool, from host: ConsentHost = .settings) {
        guard canToggle else { return }
        if on {
            consent.ask(from: host)
            return
        }
        consent.switchedOff()
        send(.settingSet(.dictationPolish, "off"))
    }

    func allowConsent() { consent.allow() }
    func cancelConsent() { consent.cancel() }

    /// Whether the status is a problem to show in the alert colour.
    var isProblem: Bool { consent.isProblem || keepsTimingOut }

    func apply(_ event: InkEvent) {
        switch event {
        case .engineRegistered(let engine) where engine.kind == .llm:
            models.insert(engine.id)
        case .engineUnregistered(let engine):
            models.remove(engine.id)
        case .coreStopped:
            models = []
        case .dictationStarted:
            takeTimedOut = false
        case .dictationWarningEvent(let warning) where warning.kind == .polishTimedOut:
            takeTimedOut = true
            timeoutsInARow += 1
        case .dictationInserted:
            // A take that went out without timing out, while polish was on, ends the run.
            if !takeTimedOut && isOn {
                timeoutsInARow = 0
            }
        default:
            break
        }
        consent.apply(event)
    }
}
