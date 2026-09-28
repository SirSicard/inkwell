// The user's consent for one feature that sends their words to a language model: polish (the
// dictation) or voice edit (the selection and the instruction). Each has its own.
//
// The feature is off until the user turns it on through a consent step that says where the words
// go: Apple's on-device model (they stay on this Mac) or a named cloud provider (they leave it).
// Turning it on only asks (`pending`); Allow sends `consent.allow` naming what was shown (and, for
// voice edit, the key), and the core records the consent and turns the feature on in one write.
// Cancel sends nothing. The core is the source of truth: `consent.state` says the switch, where
// the feature would send now, and whether the consent covers that. When the model moves elsewhere
// the consent no longer covers it: the feature is paused, and turning it on asks again. The core
// refuses to send meanwhile, so nothing depends on this screen being open.
//
// Each request carries a ref of its own (`consent.get:<feature>:<n>`), and only the answer to the
// newest request is applied: an older answer that arrives late never overwrites a newer state.
// That is also why engine churn (every engine.registered or engine.unregistered reads the state
// again) needs no debounce: each read is a short store query, and only the last answer counts.
//
// A later feature (a meeting's summary, Ask) is one more ConsentModel with its own words below.
import AppleEngines
import InkBridge
import Observation

@MainActor
@Observable
final class ConsentModel {
    /// Where a feature sends the user's words, as the consent step names it.
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

    /// The core's state for the feature, as this model keeps it.
    struct Snapshot: Equatable, Sendable {
        /// The feature's switch.
        var on: Bool
        /// Whether the consent covers the model the feature would use now.
        var allowed: Bool
        /// Where the feature would send now; nil with no model.
        var destination: Destination?
        /// What the core could not read ("couldn't read …").
        var error: String?

        init(on: Bool, allowed: Bool, destination: Destination?, error: String?) {
            self.on = on
            self.allowed = allowed
            self.destination = destination
            self.error = error
        }

        init(_ state: ConsentState) {
            let destination: Destination? = switch state.to {
            case .onDevice: Destination(kind: .onDevice, name: state.name ?? "")
            case .cloud: state.endpoint.map { Destination(kind: .cloud(endpoint: $0), name: state.name ?? "") }
            case nil: nil
            }
            self.init(on: state.on, allowed: state.allowed, destination: destination, error: state.error)
        }
    }

    /// The screens that can turn a feature on.
    enum Host: Equatable, Sendable {
        case settings
        case onboarding
    }

    enum Failure: Equatable, Sendable {
        case read
        case write
        /// The consent could not be recorded (the model changed while the user read, or the store
        /// refused): the feature stays as it was.
        case allow
    }

    let feature: LlmFeature
    /// The core's state (nil until read).
    private(set) var state: Snapshot?
    /// Something could not be read, recorded or saved.
    private(set) var failure: Failure?
    /// The consent step on screen: where the feature would send, waiting for Allow or Cancel.
    private(set) var pending: Destination?
    /// Which screen asked, so only that one shows the step (the first-run sheet can sit over
    /// Settings).
    private(set) var host: Host?
    /// For voice edit, the key the step turns it on with.
    private(set) var pendingKey: String?

    /// The id of the command that turns the feature's switch off (its `command.failed` carries it).
    @ObservationIgnored let switchSettingID: String
    @ObservationIgnored private let send: SendCommand
    /// The state before a switch-off the core has not confirmed, to put back if saving fails.
    @ObservationIgnored private var beforeOff: Snapshot??
    /// Requests sent so far, for their refs.
    @ObservationIgnored private var requests = 0
    /// The newest request of either kind: only its answer (or an unsolicited state) is applied.
    @ObservationIgnored private var newestRef: String?
    /// The newest `consent.get`, and the newest `consent.allow`, for their failures.
    @ObservationIgnored private var newestGet: String?
    @ObservationIgnored private var newestAllow: String?

    init(feature: LlmFeature, switchSettingID: String, send: @escaping SendCommand) {
        self.feature = feature
        self.switchSettingID = switchSettingID
        self.send = send
    }

    /// A new ref for a request of `kind` ("get" or "allow"), now the newest.
    private func nextRef(_ kind: String) -> String {
        requests += 1
        let ref = "consent.\(kind):\(feature.rawValue):\(requests)"
        newestRef = ref
        return ref
    }

    /// Where the feature would send now, if the core named a model.
    var destination: Destination? { state?.destination }

    /// On, and the consent covers the model now.
    var isAllowedOn: Bool { state?.on == true && state?.allowed == true }

    /// On, but the model now sends somewhere the user has not agreed to: nothing is sent.
    var isPaused: Bool { state?.on == true && state?.allowed == false && destination != nil }

    /// A problem to show in the alert colour.
    var isProblem: Bool { failure != nil || isPaused || state?.error != nil }

    /// The line to show for a failure or a pause, if there is one (nil: nothing wrong).
    var problem: String? {
        let what = Self.featureName(feature)
        switch failure {
        case .read: return "Couldn't read your \(what) setting. Open Settings again to retry."
        case .write: return "Couldn't save the change, so \(what) stays as it was."
        case .allow: return "Couldn't turn \(what) on, so it stays off. Try again."
        case nil: break
        }
        if let error = state?.error {
            // An unread switch counts as off, an unread consent as none.
            return "\(Self.sentence(error)), so \(what) is off or paused. Turn it on again to allow it."
        }
        if isPaused, let destination {
            return destination.isOnDevice
                ? "Paused: \(what) now uses \(destination.label). Turn it on again to allow it."
                : "Paused: \(what) would now send your words to \(destination.label). Turn it on again to allow it."
        }
        return nil
    }

    // MARK: - The consent step's words

    static func featureName(_ feature: LlmFeature) -> String {
        switch feature {
        case .polish: "polish"
        case .edit: "voice edit"
        }
    }

    static func title(_ feature: LlmFeature) -> String {
        switch feature {
        case .polish: "Turn on polish?"
        case .edit: "Turn on voice edit?"
        }
    }

    /// What the consent step says: what the feature sends, and where the words go for this model.
    static func message(_ feature: LlmFeature, _ destination: Destination) -> String {
        let what = switch feature {
        case .polish: "Polish sends what you dictate to a language model, which tidies the wording before it is typed."
        case .edit: "Voice edit sends the text you select and what you say to a language model, which rewrites the selection."
        }
        return destination.isOnDevice
            ? "\(what) It uses \(destination.label), so your words stay on this Mac."
            : "\(what) It uses \(destination.label), a cloud provider: your words leave this Mac and go to \(destination.label)."
    }

    /// The button that agrees.
    static func button(_ feature: LlmFeature, _ destination: Destination) -> String {
        guard destination.isOnDevice else { return "Send to \(destination.label)" }
        return switch feature {
        case .polish: "Turn On Polish"
        case .edit: "Turn On Voice Edit"
        }
    }

    // MARK: - Commands

    /// Reads the state from the core.
    func load() {
        let ref = nextRef("get")
        newestGet = ref
        send(.consentGet(feature, ref: ref))
    }

    /// Shows the consent step for the model now (nothing is sent until Allow). `key` is voice
    /// edit's. Does nothing while no model is named.
    func ask(from host: Host, key: String? = nil) {
        guard let destination else { return }
        failure = nil
        pending = destination
        self.host = host
        pendingKey = key
    }

    /// The consent step's Allow: the user agreed to where the feature sends. The core records it
    /// only if that is still where the model goes.
    func allow() {
        guard let destination = pending else { return }
        let key = pendingKey
        cancel()
        failure = nil
        let ref = nextRef("allow")
        newestAllow = ref
        switch destination.kind {
        case .onDevice: send(.consentAllow(feature: feature, to: .onDevice, endpoint: nil, key: key, ref: ref))
        case .cloud(let endpoint):
            send(.consentAllow(feature: feature, to: .cloud, endpoint: endpoint, key: key, ref: ref))
        }
    }

    /// The consent step's Cancel (or the step dismissed): nothing is sent.
    func cancel() {
        pending = nil
        host = nil
        pendingKey = nil
    }

    /// The owner sent the command that turns the switch off (which withdraws the consent too):
    /// shown at once, put back if the core refuses it.
    func switchedOff() {
        cancel()
        failure = nil
        if beforeOff == nil {
            beforeOff = .some(state)
        }
        state?.on = false
        state?.allowed = false
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .engineRegistered(let engine) where engine.kind == .llm:
            // Where the feature sends may have changed.
            load()
        case .engineUnregistered:
            load()
        case .coreStopped:
            cancel()
        case .consentState(let value) where value.feature == feature:
            // The answer to an older request, arriving after a newer one was sent: the newer
            // answer is the one to show. A state with no ref (after a switch-off, or a refused
            // allow) is the core's own, sent in order, and always applies.
            if let ref = value.ref, ref != newestRef { return }
            state = Snapshot(value)
            beforeOff = nil
            if failure == .read || failure == .write { failure = nil }
            // The step names a destination that is no longer the model's: close it rather than
            // change its words under the user's finger. Turning the feature on asks about the new one.
            if let pending, pending != destination {
                cancel()
            }
        case .commandFailed(let failed) where failed.id == switchSettingID && failed.command == "setting.set":
            if let before = beforeOff {
                failure = .write
                state = before
            }
            beforeOff = nil
        case .commandFailed(let failed) where failed.id != nil && failed.id == newestGet:
            failure = .read
        case .commandFailed(let failed) where failed.id != nil && failed.id == newestAllow:
            failure = .allow
        case .dictationWarningEvent(let warning) where warning.kind == .polishNotAllowed && feature == .polish:
            // The core refused to send: read where the feature goes now, so the screen says why.
            load()
        case .dictationEditFailed(let failed) where failed.reason == .notAllowed && feature == .edit:
            load()
        default:
            break
        }
    }

    /// "couldn't read x" as the start of a sentence.
    private static func sentence(_ s: String) -> String {
        s.prefix(1).uppercased() + s.dropFirst()
    }
}
