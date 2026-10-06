// Per-app call recording: what happens when an app opens the microphone for a call. Each app is
// Always (recorded at once, visibly), Ask (the consent Drop offers it) or Never, or follows the
// default for apps not chosen for. The user sets an app from the Drop ("Always for Zoom", "Never
// for Zoom") or in Settings > Meetings, which also holds the default and the list of apps the core
// has seen.
//
// The core keeps the list (meetings.calls) and answers every change with all of it; this model
// shows the last answer, and a choice in flight until its answer comes. An app is never shown by
// its identity: by the name detection saw, else as Modes names one.
import AppKit
import InkBridge
import Observation

/// What the user can choose for one app: a policy of its own, or the default.
enum CallChoice: String, CaseIterable, Identifiable, Sendable {
    case `default`
    case always
    case ask
    case never

    var id: String { rawValue }

    init(_ policy: CallPolicy) {
        switch policy {
        case .always: self = .always
        case .ask: self = .ask
        case .never: self = .never
        }
    }
}

extension CallPolicy {
    var title: String {
        switch self {
        case .always: "Always"
        case .ask: "Ask"
        case .never: "Never"
        }
    }
}

@MainActor
@Observable
final class CallPolicyModel {
    /// Where a change was asked for, so its failure shows there and nowhere else.
    enum Origin: Equatable, Sendable {
        case drop
        case settings
    }

    /// One app's row in Settings > Meetings.
    struct Row: Equatable, Identifiable {
        /// Its identity, as detection reports it. Never shown.
        let id: String
        let name: String
        let icon: NSImage?
        /// What the user chose (or default), with a choice in flight shown as made.
        let choice: CallChoice
        /// When detection last saw it hold the microphone for a call.
        let seen: Date?
    }

    /// The policy for apps not chosen for; nil until the core says.
    private(set) var defaultPolicy: CallPolicy?
    /// The apps, most recently seen first, as the core last listed them.
    private(set) var apps: [CallApp] = []
    /// Whether the core has listed them.
    private(set) var loaded = false
    /// Why the stored choices could not be read (the core's words): every app follows the default
    /// meanwhile, and a choice starts the list over, after the user confirms it.
    private(set) var unreadable: String?
    /// What the core said about the default when it started the list over (it lowered Always to
    /// Ask), shown once beside the default.
    private(set) var note: String?
    /// The last failure asked for in Settings.
    private(set) var failure: String?
    /// The last failure asked for from the Drop.
    private(set) var dropFailure: String?
    /// A choice that would start an unreadable list over, waiting for the user to confirm it.
    private(set) var startingOver: StartOver?

    /// A choice waiting for the user's yes to start the list over.
    struct StartOver: Equatable {
        let app: String
        let choice: CallChoice
    }

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let directory: any AppDirectory
    @ObservationIgnored private var nextRef = 0
    /// Each set in flight: its app, what was chosen, where, and what follows once it is saved.
    @ObservationIgnored private var inFlight: [String: InFlight] = [:]

    private struct InFlight {
        let app: String
        let choice: CallChoice
        let origin: Origin
        let saved: (@MainActor () -> Void)?
    }
    /// The app the core offers now (meeting.detected, until it is withdrawn or a meeting starts):
    /// what follows an Always saved from the Drop runs only while its offer still stands.
    @ObservationIgnored private var offered: String?
    /// The set that starts an unreadable list over, until its answer.
    @ObservationIgnored private var startOverRef: String?
    /// Choices in flight, by app: shown as made until the core answers.
    private var pending: [String: CallChoice] = [:]

    /// The id of the default's setting commands (CoreCommand gives each setting command one).
    static let defaultSettingID = "setting:\(ShellSetting.meetingsCallsDefault.rawValue)"

    init(send: @escaping SendCommand, apps: any AppDirectory = WorkspaceApps()) {
        self.send = send
        directory = apps
    }

    private func ref() -> String {
        nextRef += 1
        return "calls:\(nextRef)"
    }

    func load() {
        send(.meetingsCallsList(ref: ref()))
    }

    /// The default for apps not chosen for.
    func setDefault(_ policy: CallPolicy) {
        failure = nil
        note = nil
        defaultPolicy = policy
        send(.settingSet(.meetingsCallsDefault, policy.rawValue))
    }

    /// What happens for `app` now, when the core has listed it.
    func policy(of app: String) -> CallPolicy? {
        if let choice = pending[app] {
            return choice == .default ? defaultPolicy : Self.policy(choice)
        }
        return apps.first { $0.app == app }?.policy
    }

    /// Chooses for one app; `saved` runs once the core has saved it (never when it failed). Over
    /// a list the core cannot read, it waits for `confirmStartOver`.
    func choose(
        _ choice: CallChoice, for app: String, from origin: Origin, saved: (@MainActor () -> Void)? = nil
    ) {
        clearFailure(origin)
        if unreadable != nil {
            // Starting the list over asks first, in Settings; the Drop has no room to ask.
            switch origin {
            case .settings: startingOver = StartOver(app: app, choice: choice)
            case .drop: dropFailure = Self.unreadableFromDrop
            }
            return
        }
        set(choice, for: app, from: origin, replaceUnreadable: false, saved: saved)
    }

    /// The user agreed to start the unreadable list over with `choice`, the one the dialog showed.
    func confirmStartOver(_ choice: StartOver) {
        startingOver = nil
        set(choice.choice, for: choice.app, from: .settings, replaceUnreadable: true, saved: nil)
    }

    func cancelStartOver() {
        startingOver = nil
    }

    private func set(
        _ choice: CallChoice, for app: String, from origin: Origin, replaceUnreadable: Bool,
        saved: (@MainActor () -> Void)?
    ) {
        let ref = ref()
        if replaceUnreadable { startOverRef = ref }
        inFlight[ref] = InFlight(app: app, choice: choice, origin: origin, saved: saved)
        pending[app] = choice
        send(.meetingsCallsSet(app: app, policy: choice.rawValue, replaceUnreadable: replaceUnreadable, ref: ref))
    }

    private func clearFailure(_ origin: Origin) {
        switch origin {
        case .drop: dropFailure = nil
        case .settings: failure = nil
        }
    }

    private static func policy(_ choice: CallChoice) -> CallPolicy? {
        switch choice {
        case .default: nil
        case .always: .always
        case .ask: .ask
        case .never: .never
        }
    }

    /// The rows Settings > Meetings lists.
    var rows: [Row] {
        apps.map { app in
            // The core's name, unless it is its stand-in for an app with none ("an app").
            let name = app.appName.flatMap { $0.isEmpty || $0 == Self.nameless ? nil : $0 }
                ?? AppIdentity.label(app.app, apps: directory).name
            let choice = pending[app.app] ?? (app.chosen ? CallChoice(app.policy) : .default)
            return Row(
                id: app.app, name: name, icon: icon(app.app), choice: choice,
                seen: app.seenUnixMs.map { Date(timeIntervalSince1970: Double($0) / 1000) })
        }
    }

    /// An installed app's icon, looked up once per identity.
    private func icon(_ identity: String) -> NSImage? {
        if let known = icons[identity] { return known }
        let found = directory.app(bundleID: identity)?.icon
        icons[identity] = .some(found)
        return found
    }

    @ObservationIgnored private var icons: [String: NSImage?] = [:]

    // MARK: - Words

    /// What the core calls an app whose name and identity show nothing.
    nonisolated static let nameless = "an app"

    /// The default's row.
    static let defaultTitle = "Calls in other apps"
    static let defaultCaption = "For every app you haven't chosen for below. Ask offers to record each call; Always records it at once, and shows it is recording; Never does neither."
    /// Under a default of Always: it records without asking, so the others must be told.
    static let alwaysWarning = "Inkwell records these calls as soon as they start, without asking. Tell the people on the call that you are recording. When Inkwell can't hear an app alone, it asks instead."
    /// Under a default of Never.
    static let neverHint = "Inkwell won't offer to record calls in these apps. Record now, on Today, still records."
    static let appsTitle = "Apps"
    static let noApps = "None yet. An app shows here once it opens the microphone for a call."
    /// While the stored choices cannot be read (`why`, the core's words).
    static func unreadableLine(_ why: String) -> String {
        "Inkwell couldn't read what you chose for each app, so every app follows the default for now (Always asks first). Choosing for an app starts the list over. (\(why))"
    }
    static let unreadableFromDrop = "Inkwell couldn't read what you chose for each app. Choose in Settings > Meetings."
    static let startOverTitle = "Start the list over?"
    static let startOverDetail = "Inkwell couldn't read what you chose for each app. Saving this choice starts the list over: every other app follows the default until you choose again."

    /// What a choice reads as in an app's picker: the default names what it is now.
    func title(_ choice: CallChoice) -> String {
        switch choice {
        case .default: "Default (\((defaultPolicy ?? .ask).title))"
        case .always: CallPolicy.always.title
        case .ask: CallPolicy.ask.title
        case .never: CallPolicy.never.title
        }
    }

    /// An app's caption: when it last opened the microphone for a call.
    static func seenCaption(_ seen: Date?, calendar: Calendar = .current, now: Date = Date()) -> String? {
        guard let seen else { return nil }
        if calendar.isDate(seen, inSameDayAs: now) { return "Last call today" }
        let sameYear = calendar.component(.year, from: seen) == calendar.component(.year, from: now)
        let style = sameYear
            ? Date.FormatStyle(date: .omitted, time: .omitted).day().month(.abbreviated)
            : Date.FormatStyle(date: .omitted, time: .omitted).day().month(.abbreviated).year()
        return "Last call \(seen.formatted(style))"
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .meetingsCalls(let calls):
            defaultPolicy = calls.default
            apps = calls.apps
            loaded = true
            let message = calls.message.flatMap { $0.isEmpty ? nil : $0 }
            var answered: InFlight?
            if let ref = calls.ref, let done = inFlight.removeValue(forKey: ref) {
                if pending[done.app] == done.choice { pending[done.app] = nil }
                answered = done
            }
            if let ref = calls.ref, ref == startOverRef {
                // The list was started over and is readable again; under a default of Always the
                // answer says the default is Ask now, so the user sets Always again knowingly.
                startOverRef = nil
                unreadable = nil
                note = message
            } else {
                unreadable = message
            }
            // After the state is the answer's (an Always saved from the Drop records the call), and
            // only while the call is still offered: one that ended meanwhile is not recorded.
            if let answered, let saved = answered.saved, offered == answered.app {
                saved()
            }
        case .meetingDetected(let detected):
            // A new offer: a failure said for the last one is over.
            offered = detected.app
            dropFailure = nil
        case .meetingDetectionEnded(let ended):
            if offered == ended.app { offered = nil }
            dropFailure = nil
        case .meetingDetection(let detection) where !detection.listening:
            offered = nil
        case .meetingStarted:
            // The offer was taken (or a meeting started otherwise): its failure goes with it.
            offered = nil
            dropFailure = nil
        case .coreStopped:
            offered = nil
            // A core that starts again answers none of the old one's commands: nothing is in
            // flight, and what is stored is read again at core.ready.
            inFlight = [:]
            pending = [:]
            startOverRef = nil
            startingOver = nil
        case .settingValue(let value) where value.key == ShellSetting.meetingsCallsDefault.rawValue:
            defaultPolicy = value.value.flatMap(CallPolicy.init(rawValue:)) ?? .ask
        case .commandFailed(let failed) where failed.command == "meetings.calls.set":
            let done = failed.id.flatMap { inFlight.removeValue(forKey: $0) }
            if let done, pending[done.app] == done.choice { pending[done.app] = nil }
            if failed.id != nil, failed.id == startOverRef { startOverRef = nil }
            let words = "Couldn't save that: \(failed.message)"
            if done?.origin == .drop { dropFailure = words } else { failure = words }
            // What is stored now, whatever the failure (a list that became unreadable says so).
            load()
        case .commandFailed(let failed) where failed.command == "meetings.calls.list":
            failure = "Couldn't read the apps you chose for: \(failed.message)"
        case .commandFailed(let failed) where failed.id == Self.defaultSettingID:
            failure = "Couldn't save the default: \(failed.message)"
            // What it is, not what was asked.
            send(.settingGet(.meetingsCallsDefault))
        default:
            break
        }
    }
}
