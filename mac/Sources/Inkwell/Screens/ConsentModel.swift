// The user's consent for one feature that sends their words to a language model: polish (the
// dictation), voice edit (the selection and the instruction), or summaries and Ask (a meeting's
// transcript). Each has its own.
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
// The first run's own key asks before its model is chosen (`ask(_:from:choosing:)`): the step
// names the provider the user picked, Allow chooses it (which, for a provider off this Mac, is what
// turns local-only mode off), and the consent is sent once the core names that same destination
// (`agreed`). A choice the core refuses, or a destination it names that is not the one agreed to,
// records nothing: the agreement was for that choice only, and is dropped.
//
// Summaries and Ask are off until allowed too: the core then sends no transcript, a meeting ends
// with meeting.warning summary_not_allowed (the state is read again), and Ask answers that it
// needs the user's OK.
import AppleEngines
import Foundation
import InkBridge
import Observation

@MainActor
@Observable
final class ConsentModel {
    /// Where a feature sends the user's words, as the consent step names it.
    struct Destination: Equatable, Sendable {
        enum Kind: Hashable, Sendable {
            case onDevice
            case cloud(endpoint: String)

            /// The same destination, as the core compares them (an endpoint without its trailing
            /// slashes, which the core drops).
            func sameDestination(_ other: Kind) -> Bool {
                switch (self, other) {
                case (.onDevice, .onDevice): true
                case (.cloud(let a), .cloud(let b)): Self.trimmed(a) == Self.trimmed(b)
                default: false
                }
            }

            private static func trimmed(_ endpoint: String) -> String {
                var out = Substring(endpoint)
                while out.hasSuffix("/") { out = out.dropLast() }
                return String(out)
            }
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
        /// Every destination the user agreed the feature may send to (polish holds one per
        /// destination; Settings > AI lists them, each with Revoke).
        var consents: [Granted]

        init(on: Bool, allowed: Bool, destination: Destination?, error: String?, consents: [Granted] = []) {
            self.on = on
            self.allowed = allowed
            self.destination = destination
            self.error = error
            self.consents = consents
        }

        init(_ state: ConsentState) {
            let destination: Destination? = switch state.to {
            case .onDevice: Destination(kind: .onDevice, name: state.name ?? "")
            case .cloud: state.endpoint.map { Destination(kind: .cloud(endpoint: $0), name: state.name ?? "") }
            case nil: nil
            }
            // A cloud consent without its endpoint could not be revoked or matched: left out.
            let consents: [Granted] = (state.consents ?? []).compactMap { entry in
                switch entry.to {
                case .onDevice: Granted(kind: .onDevice, name: entry.name)
                case .cloud: entry.endpoint.map { Granted(kind: .cloud(endpoint: $0), name: entry.name) }
                }
            }
            self.init(on: state.on, allowed: state.allowed, destination: destination, error: state.error, consents: consents)
        }

        /// Whether one of the consents covers `destination` (one on this Mac covers every model
        /// on it; a cloud one, its endpoint).
        func covers(_ destination: Destination) -> Bool {
            consents.contains { $0.kind.sameDestination(destination.kind) }
        }
    }

    /// A destination the user agreed a feature may send to, as consent.state lists it.
    struct Granted: Hashable, Sendable {
        let kind: Destination.Kind
        /// For cloud, the provider's name the user agreed to.
        let name: String?

        /// The words for it in Settings > AI: "Models on this Mac", or the provider's name.
        var label: String {
            switch kind {
            case .onDevice: "Models on this Mac"
            case .cloud(let endpoint): name.flatMap { $0.isEmpty ? nil : $0 } ?? Self.host(endpoint)
            }
        }

        /// What going there means for the user's words.
        var detail: String {
            switch kind {
            case .onDevice: "Your words stay on this Mac."
            case .cloud(let endpoint): "Your words leave this Mac for \(Self.host(endpoint))."
            }
        }

        /// An endpoint's host ("api.groq.com"), or the endpoint as it is.
        static func host(_ endpoint: String) -> String {
            URL(string: endpoint)?.host() ?? endpoint
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
        /// A consent could not be revoked: it may still hold.
        case revoke
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
    /// The step on screen names a model that is not chosen yet: Allow chooses it first.
    private(set) var choosing = false
    /// The destination the user allowed before its model was chosen, waiting for the core to name
    /// it; the consent is sent then.
    private(set) var agreed: Destination?

    /// The id of the command that turns the feature's switch off (its `command.failed` carries it).
    @ObservationIgnored let switchSettingID: String
    @ObservationIgnored private let send: SendCommand
    /// What Allow does first when the step names a model not chosen yet (`choosing`): it chooses
    /// the model, and says whether it sent the choice.
    @ObservationIgnored private var choose: (@MainActor () -> Bool)?
    /// The state before a switch-off the core has not confirmed, to put back if saving fails.
    @ObservationIgnored private var beforeOff: Snapshot??
    /// Requests sent so far, for their refs.
    @ObservationIgnored private var requests = 0
    /// The newest request of either kind: only its answer (or an unsolicited state) is applied.
    @ObservationIgnored private var newestRef: String?
    /// The newest `consent.get`, and the newest `consent.allow`, for their failures.
    @ObservationIgnored private var newestGet: String?
    @ObservationIgnored private var newestAllow: String?
    @ObservationIgnored private var newestRevoke: String?

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
        // "Summaries and Ask" are two things.
        let (it, stays, isOff, uses, was) = feature == .meetings
            ? ("them", "stay", "are", "use", "they were")
            : ("it", "stays", "is", "uses", "it was")
        switch failure {
        case .read: return "Couldn't read your \(what) setting. Open Settings again to retry."
        case .write: return "Couldn't save the change, so \(what) \(stays) as \(was)."
        case .allow: return "Couldn't turn \(what) on, so \(feature == .meetings ? "they" : "it") \(stays) off. Try again."
        case .revoke: return "Couldn't revoke that, so \(what) may still send there. Try again."
        case nil: break
        }
        if let error = state?.error {
            // An unread switch counts as off, an unread consent as none.
            return "\(Self.sentence(error)), so \(what) \(isOff) off or paused. Turn \(it) on again to allow \(it)."
        }
        if isPaused, let destination {
            let words = feature == .meetings ? "meeting transcripts" : "your words"
            return destination.isOnDevice
                ? "Paused: \(what) now \(uses) \(destination.label). Turn \(it) on again to allow \(it)."
                : "Paused: \(what) would now send \(words) to \(destination.label). Turn \(it) on again to allow \(it)."
        }
        return nil
    }

    // MARK: - The consent step's words

    static func featureName(_ feature: LlmFeature) -> String {
        switch feature {
        case .polish: "polish"
        case .edit: "voice edit"
        case .meetings: "summaries and Ask"
        }
    }

    static func title(_ feature: LlmFeature) -> String {
        switch feature {
        case .polish: "Turn on polish?"
        case .edit: "Turn on voice edit?"
        case .meetings: "Turn on summaries and Ask?"
        }
    }

    /// What the consent step says: what the feature sends, and where the words go for this model.
    static func message(_ feature: LlmFeature, _ destination: Destination) -> String {
        let what = switch feature {
        case .polish: "Polish sends what you dictate to a language model, which tidies the wording before it is typed."
        case .edit: "Voice edit sends the text you select and what you say to a language model, which rewrites the selection."
        case .meetings: "Summaries and Ask send a meeting's transcript, what everyone in it said, to a language model, which writes the summary and what was promised, and answers your questions."
        }
        // A meeting's transcript holds everyone's words, not only the user's.
        let (words, stay, leave) = feature == .meetings
            ? ("the transcript", "stays", "leaves this Mac and goes")
            : ("your words", "stay", "leave this Mac and go")
        return destination.isOnDevice
            ? "\(what) It uses \(destination.label), so \(words) \(stay) on this Mac."
            : "\(what) It uses \(destination.label), a cloud provider: \(words) \(leave) to \(destination.label)."
    }

    /// What a step that also chooses its model adds: that local-only mode goes off, when the
    /// model is off this Mac (nil when it stays on).
    static func choosingNote(_ destination: Destination) -> String? {
        guard !destination.isOnDevice else { return nil }
        return "Send to \(destination.label) also chooses it as your language model and turns Local only off. Settings > AI can change both."
    }

    /// What the step on screen says: the feature's message, and for a step that also chooses its
    /// model, what choosing it changes.
    func stepMessage(_ destination: Destination) -> String {
        let message = Self.message(feature, destination)
        guard choosing, let note = Self.choosingNote(destination) else { return message }
        return "\(message) \(note)"
    }

    /// The button that agrees.
    static func button(_ feature: LlmFeature, _ destination: Destination) -> String {
        guard destination.isOnDevice else { return "Send to \(destination.label)" }
        return switch feature {
        case .polish: "Turn On Polish"
        case .edit: "Turn On Voice Edit"
        case .meetings: "Turn On Summaries and Ask"
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
        cancel()
        failure = nil
        // An agreement waiting for its choice is not this step's: it is dropped, so it can never
        // allow anything after this.
        agreed = nil
        pending = destination
        self.host = host
        pendingKey = key
    }

    /// Shows the consent step for `destination`, the model `choose` will choose: nothing is sent
    /// until Allow, which runs `choose` and sends the consent once the core names `destination`.
    /// `choose` returns whether it sent the choice (false: what it would choose is no longer what
    /// the step named).
    func ask(_ destination: Destination, from host: Host, choosing choose: @escaping @MainActor () -> Bool) {
        failure = nil
        agreed = nil
        pending = destination
        self.host = host
        pendingKey = nil
        choosing = true
        self.choose = choose
    }

    /// The consent step's Allow: the user agreed to where the feature sends. The core records it
    /// only if that is still where the model goes. A step naming a model not chosen yet chooses it
    /// first, and the consent waits for the core to name it.
    func allow() {
        guard let destination = pending else { return }
        let key = pendingKey
        let chooseFirst = choosing ? choose : nil
        cancel()
        failure = nil
        if let chooseFirst {
            // Waiting only for a choice that went: one that did not would leave an agreement that
            // a later state could act on, with no step on screen.
            if chooseFirst() {
                agreed = destination
            } else {
                failure = .allow
            }
            return
        }
        send(allowCommand(destination, key: key))
    }

    private func allowCommand(_ destination: Destination, key: String?) -> CoreCommand {
        let ref = nextRef("allow")
        newestAllow = ref
        return switch destination.kind {
        case .onDevice: .consentAllow(feature: feature, to: .onDevice, endpoint: nil, key: key, ref: ref)
        case .cloud(let endpoint): .consentAllow(feature: feature, to: .cloud, endpoint: endpoint, key: key, ref: ref)
        }
    }

    /// The user agreed, in a mode's own consent step (Settings > Modes), that polish may send to
    /// `destination`, a model a mode can pick: the core adds that consent (and turns polish's
    /// switch on), or fails if that model sends elsewhere now. Returns the command's ref, which
    /// comes back in its `consent.state` or as the id of a `command.failed`: the asker matches
    /// them and says what failed, so a failure here is not shown under the toggle too.
    @discardableResult
    func allow(forMode destination: Destination) -> String {
        let ref = nextRef("allow")
        let command: CoreCommand = switch destination.kind {
        case .onDevice: .consentAllow(feature: feature, to: .onDevice, endpoint: nil, key: nil, ref: ref)
        case .cloud(let endpoint): .consentAllow(feature: feature, to: .cloud, endpoint: endpoint, key: nil, ref: ref)
        }
        send(command)
        return ref
    }

    /// Settings > AI's Revoke: takes the consent for one destination away; the others stay.
    /// Revoking the last turns the feature off (the core's answer says so).
    func revoke(_ granted: Granted) {
        failure = nil
        let ref = nextRef("revoke")
        newestRevoke = ref
        switch granted.kind {
        case .onDevice: send(.consentRevoke(feature: feature, to: .onDevice, endpoint: nil, ref: ref))
        case .cloud(let endpoint): send(.consentRevoke(feature: feature, to: .cloud, endpoint: endpoint, ref: ref))
        }
    }

    /// The consent step's Cancel (or the step dismissed): nothing is sent.
    func cancel() {
        pending = nil
        host = nil
        pendingKey = nil
        choosing = false
        choose = nil
    }

    /// The owner sent the command that turns the switch off (which withdraws the consent too):
    /// shown at once, put back if the core refuses it.
    func switchedOff() {
        cancel()
        agreed = nil
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
            agreed = nil
        case .commandFailed(let failed) where failed.command == "llm.choose" && agreed != nil:
            // The choice the user allowed was refused: nothing changed, and nothing is allowed.
            agreed = nil
            failure = .allow
        case .consentState(let value) where value.feature == feature:
            // The answer to an older request, arriving after a newer one was sent: the newer
            // answer is the one to show. A state with no ref (after a switch-off, or a refused
            // allow) is the core's own, sent in order, and always applies.
            if let ref = value.ref, ref != newestRef { return }
            state = Snapshot(value)
            beforeOff = nil
            if failure == .read || failure == .write || failure == .revoke { failure = nil }
            // The step names a destination that is no longer the model's: close it rather than
            // change its words under the user's finger. Turning the feature on asks about the new one.
            // A step that chooses its model names one the core does not know yet: it stays.
            if let pending, !choosing, pending != destination {
                cancel()
            }
            // The model the user allowed is chosen: the consent goes, for the destination the core
            // names, only if it is the one agreed to (the core then records it only if it still is).
            // The core's own state after the choice (no ref) naming another settles it as refused;
            // an answer to an older read is not about the choice.
            if let agreed {
                if let now = destination, now.kind == agreed.kind {
                    self.agreed = nil
                    send(allowCommand(now, key: nil))
                } else if value.ref == nil {
                    self.agreed = nil
                    failure = .allow
                }
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
        case .commandFailed(let failed) where failed.id != nil && failed.id == newestRevoke:
            failure = .revoke
        case .dictationWarningEvent(let warning) where warning.kind == .polishNotAllowed && feature == .polish:
            // The core refused to send: read where the feature goes now, so the screen says why.
            load()
        case .dictationEditFailed(let failed) where failed.reason == .notAllowed && feature == .edit:
            load()
        case .meetingWarningEvent(let warning) where warning.kind == .summaryNotAllowed && feature == .meetings:
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
