// The Polish toggle (Settings > AI): the user's wish, and whether a language model can do it.
//
// The toggle reads "on" only when both hold: the user turned it on, and the core has a working
// language model registered for polish (engine.registered, kind llm, and not let go of since).
// With no working model it reads off, cannot be switched, and says why, from what the Apple
// engines found (Apple Intelligence off, not ready, or not on this Mac). A toggle that read "on"
// while nothing could polish would promise what the app cannot do.
//
// It also tells "polish keeps timing out" from an ordinary failure: a take whose polish ran out of
// its time budget arrives as the dictation warning polish_timed_out, which the core sends once the
// pipeline tells a timeout from a cancel (until then a timeout reads as polish_failed).
import AppleEngines
import InkBridge
import Observation

@MainActor
@Observable
final class PolishModel {
    /// What the Apple engines found for polish, once they registered (nil before).
    private(set) var appleState: AppleEngineState?
    /// The user's wish, from the core's store (nil until read).
    private(set) var preference: Bool?
    /// Language models the core confirmed and still holds, by id.
    private(set) var models: Set<String> = []
    /// Polish timeouts in a row, across takes; a take that polished resets it.
    private(set) var timeoutsInARow = 0

    /// Timeouts in a row at which the toggle warns.
    static let timeoutWarning = 2

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var takeTimedOut = false

    init(send: @escaping SendCommand) {
        self.send = send
    }

    /// Whether a language model can polish now.
    var hasWorkingEngine: Bool { !models.isEmpty }

    /// What the toggle shows: on only with the wish and a working engine.
    var isOn: Bool { preference == true && hasWorkingEngine }

    /// Whether the toggle can be switched.
    var canToggle: Bool { hasWorkingEngine && preference != nil }

    /// Polish has timed out on several takes in a row.
    var keepsTimingOut: Bool { timeoutsInARow >= Self.timeoutWarning }

    /// The line under the toggle.
    var status: String {
        guard hasWorkingEngine else { return Self.unavailable(appleState) }
        if keepsTimingOut {
            return "Polish keeps timing out, so your words go in as you said them."
        }
        return isOn
            ? "On this Mac, with Apple Intelligence. Nothing leaves it."
            : "Off. Your words go in as you said them."
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

    /// Reads the wish from the core's store.
    func load() {
        send(.settingGet(.dictationPolish))
    }

    /// The Apple engines registered polish, or found it could not run.
    func appleEnginesReported(_ state: AppleEngineState) {
        appleState = state
    }

    /// The user switched the toggle. Does nothing without a working engine.
    func setOn(_ on: Bool) {
        guard canToggle else { return }
        preference = on
        send(.settingSet(.dictationPolish, on ? "on" : "off"))
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .engineRegistered(let engine) where engine.kind == .llm:
            models.insert(engine.id)
        case .engineUnregistered(let engine):
            models.remove(engine.id)
        case .coreStopped:
            models = []
        case .settingValue(let value) where value.key == ShellSetting.dictationPolish.rawValue:
            // Never set: off, as a mode's polish is by default.
            preference = value.value == "on"
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
    }
}
