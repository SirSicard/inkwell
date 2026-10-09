// Settings > Modes: each mode with how it writes, the apps it is picked for (each by its name and
// icon) and the language model it polishes on; and the mode editor (ModesEditor.swift), which adds,
// changes and deletes modes through the core (modes.save, modes.delete).
//
// A mode names apps by identity, as the core matches them: a bundle id (com.example.mail), or part
// of one. That identity is never shown. An installed app is named by NSWorkspace (its display name
// and icon); one that is not installed is named from a short list of well-known apps, and failing
// that as "an app not on this Mac". A fragment that is not an id ("slack") is shown as a word.
//
// The core is the source of truth. Every answer (modes.listed, to a list, a save or a delete) is the
// whole list as stored, sent in order, so the newest one is what shows. A save's own answer (matched
// by its ref) closes the editor; a refusal keeps the editor open with words chosen by its code, never
// the core's message. The user's words travel here (names, instructions, apps): never log them.
//
// Polish. A mode is polished only with its own switch on, Settings > AI's "Polish my words" on, a
// model to send to (the AI setting's, or the mode's own: polish_model), and the user's OK for where
// that model sends (one per destination). No "Polish" chip shows without a model; with one that
// can't be used now, a dimmed "Polish · off" says why. A mode's own model that the core does not
// hold now (missing), or that sends elsewhere than where it did when the mode was saved (moved, or
// never recorded: unrecorded), is said under the row, with Confirm… for the last two: polish never
// goes to a destination the user did not agree to for that mode. A model at a destination no
// consent covers asks for its own OK (consent.allow) before the mode is saved on it.
import AppKit
import InkBridge

/// An app as the screen shows it.
struct AppLabel: Equatable, Identifiable {
    /// The name the user knows it by. Never a bundle id.
    let name: String
    /// Its icon, when it is installed.
    let icon: NSImage?
    /// Whether it is on this Mac.
    let installed: Bool

    /// Unique within one mode's list.
    let id: String
}

/// Finds installed apps.
protocol AppDirectory {
    /// The installed app with this bundle id: its display name and icon.
    func app(bundleID: String) -> (name: String, icon: NSImage)?
}

/// Launch Services' answer, through NSWorkspace.
struct WorkspaceApps: AppDirectory {
    func app(bundleID: String) -> (name: String, icon: NSImage)? {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else {
            return nil
        }
        var name = FileManager.default.displayName(atPath: url.path)
        if name.hasSuffix(".app") {
            name.removeLast(4)
        }
        return (name, NSWorkspace.shared.icon(forFile: url.path))
    }
}

/// Names an app identity for the screen.
enum AppIdentity {
    /// Well-known apps, for an identity whose app is not installed here.
    static let known: [String: String] = [
        "com.apple.MobileSMS": "Messages",
        "com.apple.mail": "Mail",
        "com.apple.Notes": "Notes",
        "com.apple.Terminal": "Terminal",
        "com.apple.TextEdit": "TextEdit",
        "com.apple.Safari": "Safari",
        "com.apple.dt.Xcode": "Xcode",
        "com.tinyspeck.slackmacgap": "Slack",
        "net.whatsapp.WhatsApp": "WhatsApp",
        "com.microsoft.VSCode": "VS Code",
        "com.microsoft.teams2": "Microsoft Teams",
        "com.microsoft.Outlook": "Outlook",
        "com.microsoft.Word": "Word",
        "us.zoom.xos": "Zoom",
        "com.google.Chrome": "Chrome",
        "com.googlecode.iterm2": "iTerm",
        "com.hnc.Discord": "Discord",
        "ru.keepcoder.Telegram": "Telegram",
        "notion.id": "Notion",
        "com.linear": "Linear",
        "com.figma.Desktop": "Figma",
    ]

    /// Whether `identity` looks like a bundle id rather than a word.
    static func isBundleID(_ identity: String) -> Bool {
        identity.contains(".") && !identity.contains(" ")
    }

    /// `identity` as the user should see it.
    static func label(_ identity: String, apps: any AppDirectory) -> AppLabel {
        let trimmed = identity.trimmingCharacters(in: .whitespaces)
        if let app = apps.app(bundleID: trimmed) {
            return AppLabel(name: app.name, icon: app.icon, installed: true, id: trimmed)
        }
        if let name = known[trimmed] ?? known.first(where: { $0.key.caseInsensitiveCompare(trimmed) == .orderedSame })?.value {
            return AppLabel(name: name, icon: nil, installed: false, id: trimmed)
        }
        if isBundleID(trimmed) {
            return AppLabel(name: "An app not on this Mac", icon: nil, installed: false, id: trimmed)
        }
        // A fragment the core matches inside identities: shown as the word it is.
        let word = trimmed.prefix(1).uppercased() + trimmed.dropFirst()
        return AppLabel(name: word, icon: nil, installed: false, id: trimmed)
    }

    /// Whether two identities name the same app, as the editor compares them (the core matches
    /// a mode's app inside the frontmost app's identity, ignoring case).
    static func same(_ a: String, _ b: String) -> Bool {
        a.trimmingCharacters(in: .whitespaces).caseInsensitiveCompare(b.trimmingCharacters(in: .whitespaces)) == .orderedSame
    }
}

/// An app the editor offers to add: one running now, or one the user chose in /Applications.
struct PickedApp: Equatable, Identifiable {
    /// Its bundle id, as a mode names it. Never shown.
    let identity: String
    let name: String
    let icon: NSImage?

    var id: String { identity }

    /// The app at `url` (a .app the user chose), or nil when it has no bundle id: nothing could
    /// tell when it is in front.
    static func at(_ url: URL) -> PickedApp? {
        guard let identity = Bundle(url: url)?.bundleIdentifier, !identity.isEmpty else { return nil }
        var name = FileManager.default.displayName(atPath: url.path)
        if name.hasSuffix(".app") {
            name.removeLast(4)
        }
        return PickedApp(identity: identity, name: name, icon: NSWorkspace.shared.icon(forFile: url.path))
    }
}

/// The apps running now, for the editor's "Running now".
protocol RunningApps {
    /// Apps with a window and a Dock icon (regular apps), Inkwell left out, by name.
    func running() -> [PickedApp]
}

/// NSWorkspace's running apps.
struct WorkspaceRunningApps: RunningApps {
    /// Inkwell's own bundle id: dictating into Inkwell needs no mode.
    static let inkwell = "com.inkwell.app"

    func running() -> [PickedApp] {
        var seen = Set<String>()
        let own = [Self.inkwell, Bundle.main.bundleIdentifier].compactMap { $0?.lowercased() }
        return NSWorkspace.shared.runningApplications.compactMap { app -> PickedApp? in
            guard app.activationPolicy == .regular, let identity = app.bundleIdentifier,
                  !own.contains(identity.lowercased()), seen.insert(identity.lowercased()).inserted
            else { return nil }
            let name = app.localizedName ?? AppIdentity.label(identity, apps: WorkspaceApps()).name
            return PickedApp(identity: identity, name: name, icon: app.icon)
        }
        .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}

/// A language model a mode can be polished on, as modes.listed lists it (polish_models).
struct ModelChoice: Equatable, Sendable {
    /// Its id as a mode names it (engine:<id>, provider:<id>). Never shown.
    let id: String
    /// Where it sends a dictation, as a consent names it.
    let destination: ConsentModel.Destination
    /// The model it asks for (for the own-key provider, the one chosen in AI).
    let model: String?
    /// A polish consent covers it, and local-only mode lets it.
    let allowed: Bool
    /// Local-only mode is on and it is not on this Mac.
    let blockedLocalOnly: Bool

    init(id: String, destination: ConsentModel.Destination, model: String?, allowed: Bool, blockedLocalOnly: Bool) {
        self.id = id
        self.destination = destination
        self.model = model
        self.allowed = allowed
        self.blockedLocalOnly = blockedLocalOnly
    }

    init(_ choice: LanguageModelChoice) {
        let kind: ConsentModel.Destination.Kind = switch choice.to {
        case .onDevice: .onDevice
        case .cloud: .cloud(endpoint: choice.endpoint ?? "")
        }
        self.init(
            id: choice.id, destination: ConsentModel.Destination(kind: kind, name: choice.name), model: choice.model,
            allowed: choice.allowed, blockedLocalOnly: choice.blockedLocalOnly ?? false)
    }

    /// The own-key provider: a mode may ask it for another model by name.
    var isProvider: Bool { id.hasPrefix("provider:") }

    /// What the editor and the rows call it: "Apple's on-device model", "llama3.2, on this Mac",
    /// or for a cloud provider its name and the model it asks for ("Groq · llama-3.1-8b-instant").
    /// `modelName` is a mode's own model at the provider.
    func label(modelName: String? = nil) -> String {
        let own = modelName.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }
        switch destination.kind {
        case .onDevice:
            guard let own, isProvider else { return destination.label }
            return ConsentModel.Destination(kind: .onDevice, name: own).label
        case .cloud:
            guard let model = own ?? model, !model.isEmpty else { return destination.label }
            return "\(destination.label) · \(model)"
        }
    }

    /// polish_model_confirm_to: where the user agreed it sends.
    var confirmFields: [String: Any] {
        switch destination.kind {
        case .onDevice: ["to": "on_device"]
        case .cloud(let endpoint): ["to": "cloud", "endpoint": endpoint]
        }
    }
}

/// Whether a mode's dictations are polished now, and if not, why.
enum PolishState: Equatable, Sendable {
    /// The mode's own switch is off.
    case notWanted
    /// No language model at all: the AI setting has none.
    case noModel
    /// The mode's own model is not held now (let go of, or another provider chosen).
    case missing(name: String)
    /// The mode's own model sends elsewhere than when it was saved (`moved`), or where was never
    /// recorded: the user confirms it before it gets anything.
    case confirm(ModelChoice, label: String, moved: Bool)
    /// Local-only mode is on, and the model is off this Mac.
    case localOnly(ModelChoice, label: String)
    /// Settings > AI's "Polish my words" is off.
    case switchedOff
    /// No polish consent covers where the model sends. `own`: the mode's own model, whose OK is
    /// asked here; else the AI setting's, asked in AI.
    case needsOK(ModelChoice, label: String, own: Bool)
    /// Polished, on this model.
    case ready(ModelChoice, label: String)

    /// The chip: "Polish" when it runs, "Polish · off" when it is wanted and a model is there but
    /// can't be used now, and none without a model (or without the mode's switch).
    var chip: String? {
        switch self {
        case .notWanted, .noModel, .missing: nil
        case .ready: "Polish"
        case .confirm, .localOnly, .switchedOff, .needsOK: "Polish \u{00B7} off"
        }
    }

    /// Whether the chip is dimmed: polish is wanted, and off.
    var chipDimmed: Bool { chip != nil && !isReady }

    var isReady: Bool {
        if case .ready = self { true } else { false }
    }

    /// Why the chip is off, for its tooltip and VoiceOver.
    var why: String? {
        switch self {
        case .notWanted, .noModel, .missing, .ready: nil
        case .confirm: "Confirm where its model sends"
        case .localOnly: "Local only is on in AI"
        case .switchedOff: "Polish my words is off in AI"
        case .needsOK(let choice, _, own: true): "Polish needs your OK to send to \(choice.destination.label)"
        case .needsOK(_, _, own: false): "Polish needs your OK again in AI"
        }
    }

    /// The chip as VoiceOver reads it.
    var spokenChip: String? {
        guard let chip else { return nil }
        guard let why else { return chip }
        return "Polish, off: \(why)"
    }

    /// What a row says under its chips about its own model, and what its button does.
    enum Fix: Equatable, Sendable {
        /// Confirm… : where the model sends now, agreed to.
        case confirm
        /// Allow… : polish's OK for where the model sends.
        case allow
    }

    var rowNote: (text: String, fix: Fix?)? {
        switch self {
        case .missing(let name):
            ("Its model, \(name), isn't available now, so this mode isn't polished. Edit it to pick another.", nil)
        case .confirm(_, let label, moved: true):
            ("Its model now sends somewhere else: \(label). Confirm it to polish with it again.", .confirm)
        case .confirm(_, let label, moved: false):
            ("Where its model sends was never recorded: \(label). Confirm it to polish with it.", .confirm)
        case .localOnly(_, let label):
            ("Local only is on, so nothing goes to \(label). Turn Local only off in AI to use it.", nil)
        case .needsOK(let choice, _, own: true):
            ("Polish needs your OK to send this mode's words to \(choice.destination.label).", .allow)
        case .notWanted, .noModel, .switchedOff, .needsOK(_, _, own: false), .ready:
            nil
        }
    }
}

/// One mode's row.
struct ModeRow: Equatable, Identifiable {
    let id: String
    /// Its name, as the user named it (and says it).
    let name: String
    let isDefault: Bool
    /// What the row is titled: "Everywhere else" for the default mode beside others.
    let title: String
    /// How it writes: its style, then "Clean up speech" where on.
    let traits: [String]
    /// Whether it is polished now, and if not, why.
    let polish: PolishState
    /// The apps it is picked for, by name.
    let apps: [AppLabel]
}

/// A save, as modes.save sends it: only the fields set are named (a field left nil keeps its
/// value), so an edit never overwrites what it did not change.
struct ModeSave: Equatable, Sendable {
    /// Absent to add a mode.
    var id: String?
    var name: String?
    /// Never "other": a style this build does not know is kept by leaving it out.
    var style: ModeStyle?
    var polish: Bool?
    var removeFillers: Bool?
    var polishPrompt: String?
    var apps: [String]?
    /// .some(nil) sends null: the AI setting's model.
    var polishModel: String??
    /// .some(nil) sends null: the model chosen with the provider in AI.
    var polishModelName: String??
    /// Confirms where the mode's model sends now: the destination the user agreed to.
    var confirmTo: ModelChoice?
    /// Moves an app another mode has (the editor said "Moves Slack from Chat.").
    var takeApps = false
    /// Replaces stored modes that can't be read (the user chose Start over).
    var replaceUnreadable = false

    /// The "mode" object.
    var modeFields: [String: Any] {
        var out: [String: Any] = [:]
        if let id { out["id"] = id }
        if let name { out["name"] = name }
        if let style, style != .other { out["style"] = style.rawValue }
        if let polish { out["polish"] = polish }
        if let removeFillers { out["remove_fillers"] = removeFillers }
        if let polishPrompt { out["polish_prompt"] = polishPrompt }
        if let apps { out["apps"] = apps }
        if let polishModel { out["polish_model"] = polishModel ?? NSNull() }
        if let polishModelName { out["polish_model_name"] = polishModelName ?? NSNull() }
        if let confirmTo {
            out["polish_model_confirm"] = true
            out["polish_model_confirm_to"] = confirmTo.confirmFields
        }
        return out
    }
}

/// The editor's draft of one mode: what the sheet shows and changes. The model saves it.
@MainActor
@Observable
final class ModeEditor: Identifiable {
    /// The mode edited; nil while adding one.
    let modeID: String?
    let isDefault: Bool
    /// The mode as it was listed when the editor opened (nil while adding).
    let original: ModeInfo?

    var name: String
    var style: ModeStyle
    var removeFillers: Bool
    var polish: Bool
    /// The mode's polish instructions; blank uses the default.
    var prompt: String
    /// The apps it is picked for, by identity.
    var apps: [String]
    /// The mode's own model, by id; nil for the AI setting's.
    var polishModel: String?
    /// A model at the provider (provider: models only); blank for the one chosen in AI.
    var polishModelName: String
    /// Where the user confirmed the mode's model sends now (it had moved, or was never recorded):
    /// the destination they were shown, which the save sends back as it was, never one looked up
    /// again at the save.
    var confirmedTo: ModelChoice?
    /// The user confirmed it.
    var confirmed: Bool { confirmedTo != nil }

    /// Why the last save was refused, in words to show (nil: nothing wrong).
    var error: String?
    /// A save (or the OK it asked for first) is on its way.
    var saving = false
    /// The OK for where the mode's model sends, on screen before saving: Allow records it, then saves.
    var consentStep: ConsentModel.Destination?
    /// The core refused an app another mode has: the next save moves it (that save only).
    var takeApps = false

    nonisolated var id: ObjectIdentifier { ObjectIdentifier(self) }

    init(mode: ModeInfo?, isDefault: Bool) {
        modeID = mode?.id
        self.isDefault = isDefault
        original = mode
        name = mode?.name ?? ""
        // A new mode writes as the built-in default does.
        style = mode?.style ?? .formal
        removeFillers = mode?.removeFillers ?? true
        polish = mode?.polish ?? false
        prompt = mode?.polishPrompt ?? ""
        apps = mode?.apps ?? []
        polishModel = mode?.polishModel
        polishModelName = mode?.polishModelName ?? ""
    }

    var adding: Bool { modeID == nil }

    /// The name changed from the one saved: voice commands that said the old one no longer do.
    var renamed: Bool {
        guard let original else { return false }
        return name.trimmingCharacters(in: .whitespacesAndNewlines) != original.name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The model picked differs from the one saved (by id, or by name at the provider).
    var pinChanged: Bool {
        guard let original else { return polishModel != nil }
        return polishModel != original.polishModel || modelNameToSend != original.polishModelName
    }

    /// The model name as saved: none for a model that is not a provider's, or when blank.
    var modelNameToSend: String? {
        guard polishModel?.hasPrefix("provider:") == true else { return nil }
        let trimmed = polishModelName.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// "120 / 2,000", counted as the core counts (Unicode scalars, as Rust's chars).
    var promptCount: String {
        "\(prompt.unicodeScalars.count.formatted()) / \(ModesModel.promptLimit.formatted())"
    }

    var promptTooLong: Bool { prompt.unicodeScalars.count > ModesModel.promptLimit }
}

@MainActor
@Observable
final class ModesModel {
    private(set) var rows: [ModeRow] = []
    /// The modes could not be read: nothing can be changed, until the user starts over.
    private(set) var failed = false
    /// The stored modes can't be read (a list that failed, or a save refused with list_unreadable):
    /// Start over is offered.
    private(set) var unreadable = false
    /// What a delete, a confirm or a start-over could not do, in words (nil: nothing wrong).
    private(set) var problem: String?
    /// The models a mode can pick now, the AI setting's first.
    private(set) var choices: [ModelChoice] = []
    /// The AI setting's model (in `choices`), if there is one.
    private(set) var settingModel: String?
    /// The instructions a mode with blank ones uses: the editor's placeholder.
    private(set) var defaultPrompt = ""
    /// The editor on screen.
    var editor: ModeEditor?
    /// The mode whose delete is being confirmed.
    var deleting: ModeRow?
    /// A row's Confirm… or Allow…, on screen.
    var confirming: Confirming?
    /// Start over's confirmation, on screen.
    var confirmingStartOver = false

    /// A row's Confirm… (where its model sends now) or Allow… (polish's OK for it).
    struct Confirming: Identifiable, Equatable {
        let modeID: String
        let modeName: String
        let choice: ModelChoice
        let label: String
        /// Confirms the mode's model too (it had moved, or was never recorded); else only the OK.
        let pin: Bool
        /// The OK is asked too: no polish consent covers where it sends.
        var asksOK: Bool { !choice.allowed && !choice.blockedLocalOnly }

        var id: String { modeID }
    }

    /// The longest polish instructions the core keeps.
    static let promptLimit = 2000
    static let refPrefix = "modes:"
    static let loadFailedText = "Your modes could not be read."

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let apps: any AppDirectory
    @ObservationIgnored let running: any RunningApps
    /// Polish's consents: a mode's own OK is recorded there (one per destination).
    @ObservationIgnored private let consent: ConsentModel?
    /// Settings > AI's "Polish my words", from the core (nil until read).
    @ObservationIgnored private let polishSwitch: @MainActor () -> Bool?
    /// The latest listing.
    @ObservationIgnored private var listed: ModesListed?
    @ObservationIgnored private var requests = 0
    /// Asked for, or listed, at least once: only then do changes elsewhere read again.
    @ObservationIgnored private var loaded = false
    /// What is waiting for an answer, by ref.
    /// Saves waiting for the core, each with the editor that sent it: a refusal that arrives once
    /// its editor has closed is said in the section (another editor may have saved meanwhile).
    @ObservationIgnored private var pendingSaves: [String: ModeEditor] = [:]
    /// Apps as shown, by identity (lowercased): Launch Services is asked once per app.
    @ObservationIgnored private var labelCache: [String: AppLabel] = [:]
    @ObservationIgnored private var pendingDelete: (ref: String, name: String)?
    @ObservationIgnored private var pendingConfirm: String?
    @ObservationIgnored private var pendingStartOver: String?
    /// A row's delete, confirm or OK, or Start over, is waiting for the core: the rows' buttons
    /// wait too, so a second tap never takes the first one's place.
    private(set) var busy = false
    /// An OK on its way, and what follows it once the core has recorded it.
    @ObservationIgnored private var pendingOK: (ref: String, destination: ConsentModel.Destination, then: AfterOK)?

    private struct SaveApproval {
        let editor: ModeEditor
        let choice: ModelChoice
        let modelName: String?

        @MainActor func matches(_ current: ModeEditor?, choices: [ModelChoice]) -> Bool {
            guard current === editor, editor.polish,
                  editor.polishModel == choice.id, editor.modelNameToSend == modelName,
                  let now = choices.first(where: { $0.id == choice.id }) else { return false }
            return now.destination == choice.destination && now.model == choice.model && !now.blockedLocalOnly
        }
    }
    @ObservationIgnored private var saveApproval: SaveApproval?

    private enum AfterOK {
        case saveEditor(SaveApproval)
        case confirm(Confirming)
        case nothing
    }

    init(
        send: @escaping SendCommand, apps: any AppDirectory = WorkspaceApps(),
        running: any RunningApps = WorkspaceRunningApps(), consent: ConsentModel? = nil,
        polishSwitch: @escaping @MainActor () -> Bool? = { nil }
    ) {
        self.send = send
        self.apps = apps
        self.running = running
        self.consent = consent
        self.polishSwitch = polishSwitch
    }

    private func nextRef() -> String {
        requests += 1
        return "\(Self.refPrefix)\(requests)"
    }

    func load() {
        loaded = true
        send(.modesList(ref: nextRef()))
    }

    static func style(_ style: ModeStyle) -> String {
        switch style {
        case .formal: "Formal"
        case .casual: "Casual"
        case .relaxed: "Relaxed"
        case .other: "Own style"
        }
    }

    // MARK: - What the rows and the editor show

    /// The default mode's row title beside other modes.
    static let everywhereElse = "Everywhere else"

    /// A mode's own model when the core does not hold it: named from its id, never shown as it is.
    static func pinName(_ id: String, modelName: String?) -> String {
        let own = modelName.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }
        if id.hasPrefix("provider:") {
            let provider = CloudModel.name(String(id.dropFirst("provider:".count)))
            return own.map { "\(provider) \u{00B7} \($0)" } ?? provider
        }
        if id == "engine:apple-foundation-models" {
            return "Apple's on-device model"
        }
        return "a model that was on this Mac"
    }

    /// Whether a mode with these settings is polished now, and if not, why. `ignoringSwitch`: as
    /// if Settings > AI's switch were on (what the editor says of the model alone).
    func polishState(
        polish: Bool, pin: String?, modelName: String?, pinState: PolishModelState?, ignoringSwitch: Bool = false
    ) -> PolishState {
        guard polish else { return .notWanted }
        let choice: ModelChoice
        let label: String
        if let pin {
            guard pinState != .missing, let found = choices.first(where: { $0.id == pin }) else {
                return .missing(name: Self.pinName(pin, modelName: modelName))
            }
            label = found.label(modelName: modelName)
            if pinState == .moved || pinState == .unrecorded {
                return .confirm(found, label: label, moved: pinState == .moved)
            }
            choice = found
        } else {
            guard let id = settingModel, let found = choices.first(where: { $0.id == id }) else { return .noModel }
            choice = found
            label = found.label()
        }
        if choice.blockedLocalOnly { return .localOnly(choice, label: label) }
        // Unread, the switch is not called off: the core's state arrives at launch.
        if !ignoringSwitch, polishSwitch() == false { return .switchedOff }
        if !choice.allowed { return .needsOK(choice, label: label, own: pin != nil) }
        return .ready(choice, label: label)
    }

    /// The editor's draft, as a take would find it once saved: a model just picked (or confirmed)
    /// is recorded where it sends now.
    func polishState(_ editor: ModeEditor, polish: Bool? = nil, ignoringSwitch: Bool = false) -> PolishState {
        let state: PolishModelState? = editor.pinChanged || confirmed(editor) ? nil : editor.original?.polishModelState
        return polishState(
            polish: polish ?? editor.polish, pin: editor.polishModel, modelName: editor.modelNameToSend, pinState: state,
            ignoringSwitch: ignoringSwitch)
    }

    /// Whether the editor's confirmation still holds: the destination confirmed is where the model
    /// sends now (a listing since may say it moved again; the save would then be refused).
    func confirmed(_ editor: ModeEditor) -> Bool {
        guard let shown = editor.confirmedTo else { return false }
        return choices.first { $0.id == shown.id }?.destination == shown.destination
    }

    /// The line under the editor's Polish switch, when the mode wants polish and nothing would
    /// polish it whatever its model.
    func polishNote(_ editor: ModeEditor) -> String? {
        switch polishState(editor) {
        case .noModel: "No language model is available on this Mac, so nothing is polished."
        case .switchedOff: "Polish my words is off in AI, so nothing is polished."
        case .needsOK(_, _, own: false): "Polish needs your OK again in AI, so nothing is polished."
        default: nil
        }
    }

    /// What the editor says under its model: where it sends, or what stops it.
    func modelNote(_ editor: ModeEditor) -> (text: String, isProblem: Bool) {
        // The model alone: the mode's switch and AI's are said under the Polish switch.
        let state = polishState(editor, polish: true, ignoringSwitch: true)
        let asks = editor.polish && (editor.pinChanged || confirmed(editor) || !(editor.original?.polish ?? false))
        switch state {
        case .missing:
            return ("This model isn't available now, so this mode isn't polished. Pick another.", true)
        case .confirm(_, let label, let moved):
            return (moved
                ? "Its model now sends somewhere else: \(label). Confirm it to polish with it again."
                : "Where its model sends was never recorded: \(label). Confirm it to polish with it.", true)
        case .localOnly:
            return ("Local only is on, so nothing goes to it. Turn Local only off in AI first.", true)
        case .needsOK(let choice, _, own: true) where asks:
            return ("Saving asks for your OK to send this mode's words to \(choice.destination.label).", false)
        case .needsOK(let choice, _, own: true):
            return ("Polish needs your OK to send this mode's words to \(choice.destination.label).", true)
        case .noModel:
            return ("Uses the model chosen in AI. There is none now.", false)
        case .ready(let choice, _), .needsOK(let choice, _, _):
            let prefix = editor.polishModel == nil ? "Uses the model chosen in AI. " : ""
            return (prefix + (choice.destination.isOnDevice
                ? "Your words stay on this Mac."
                : "Your words go to \(choice.destination.label)."), false)
        case .notWanted, .switchedOff:
            // Neither, with the mode's switch taken as on and AI's ignored.
            return ("Uses the model chosen in AI.", false)
        }
    }

    /// The editor's model picker: the AI setting's first, then each model, and a mode's own model
    /// the core no longer holds, so the picker shows what is saved.
    func modelOptions(_ editor: ModeEditor) -> [(id: String?, label: String)] {
        let setting = settingModel.flatMap { id in choices.first { $0.id == id } }
        var options: [(id: String?, label: String)] = [
            (nil, setting.map { "As in AI (\($0.label()))" } ?? "As in AI (none now)"),
        ]
        options += choices.map { (Optional($0.id), $0.label()) }
        if let pin = editor.polishModel, !choices.contains(where: { $0.id == pin }) {
            options.append((pin, "\(Self.pinName(pin, modelName: nil)) (not available)"))
        }
        return options
    }

    /// The mode other than `modeID` that has `identity`, by its name.
    func owner(of identity: String, excluding modeID: String?) -> String? {
        guard let listed else { return nil }
        return listed.modes.first { mode in
            mode.id != modeID && mode.apps.contains { AppIdentity.same($0, identity) }
        }.map { $0.id == listed.defaultId && listed.modes.count > 1 ? Self.everywhereElse : $0.name }
    }

    /// An app as the rows and the editor show it, asked once per app.
    func label(_ identity: String) -> AppLabel {
        let key = identity.trimmingCharacters(in: .whitespaces).lowercased()
        if let known = labelCache[key], known.id == identity.trimmingCharacters(in: .whitespaces) {
            return known
        }
        let label = AppIdentity.label(identity, apps: apps)
        labelCache[key] = label
        return label
    }

    /// "Moves Slack from Chat." for each app the draft takes from another mode.
    func movingNotes(_ editor: ModeEditor) -> [String] {
        editor.apps.filter { identity in
            !(editor.original?.apps.contains { AppIdentity.same($0, identity) } ?? false)
        }.compactMap { identity in
            owner(of: identity, excluding: editor.modeID).map { "Moves \(label(identity).name) from \($0)." }
        }
    }

    /// The running apps the editor can add: not in the draft already, each with the mode it is in.
    func runningOffers(_ editor: ModeEditor) -> [(app: PickedApp, owner: String?)] {
        running.running()
            .filter { app in !editor.apps.contains { AppIdentity.same($0, app.identity) } }
            .map { ($0, owner(of: $0.identity, excluding: editor.modeID)) }
    }

    // MARK: - The editor's words

    /// The model the own-key provider asks for unless the mode names another (the field's placeholder).
    func providerModel(_ editor: ModeEditor) -> String? {
        guard let pin = editor.polishModel else { return nil }
        return choices.first { $0.id == pin }?.model
    }

    /// Whether the editor offers Confirm: the mode's own model, unchanged, sends elsewhere than when
    /// it was saved, or where was never recorded.
    func canConfirmInEditor(_ editor: ModeEditor) -> Bool {
        guard !confirmed(editor), !editor.pinChanged else { return false }
        if case .confirm = polishState(editor, polish: true, ignoringSwitch: true) { return true }
        return false
    }

    /// The editor's OK step: "Polish “Chat” with Groq?".
    func consentTitle(_ editor: ModeEditor) -> String {
        let name = editor.name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard case .needsOK(_, let label, own: true) = polishState(editor, ignoringSwitch: true), !name.isEmpty else {
            return PolishModel.consentTitle
        }
        return "Polish \u{201C}\(name)\u{201D} with \(label)?"
    }

    /// What the OK step says: where the words go, and that it turns polish on if it is off.
    func consentMessage(_ destination: ConsentModel.Destination) -> String {
        let message = ConsentModel.message(.polish, destination)
        return polishSwitch() == false ? message + " This also turns on Polish my words." : message
    }

    /// An app the user chose in the open panel.
    func addChosen(_ url: URL, to editor: ModeEditor) {
        guard let app = PickedApp.at(url) else {
            editor.error = "That app has no bundle identifier, so Inkwell can't tell when it's in front."
            return
        }
        guard app.identity.caseInsensitiveCompare(WorkspaceRunningApps.inkwell) != .orderedSame else {
            editor.error = "Inkwell itself needs no mode."
            return
        }
        addApp(app.identity, to: editor)
    }

    // MARK: - The editor

    /// Opens the editor on a new mode.
    func add() {
        guard !failed else { return }
        editor = ModeEditor(mode: nil, isDefault: false)
    }

    /// Opens the editor on a mode.
    func edit(_ id: String) {
        guard let listed, let mode = listed.modes.first(where: { $0.id == id }) else { return }
        editor = ModeEditor(mode: mode, isDefault: id == listed.defaultId)
    }

    /// Cancel, Escape or the sheet closed: nothing more is saved. A save already sent still lands;
    /// if the core refuses it, the section says so (the editor is gone). An OK still on its way is
    /// recorded, but saves nothing.
    func closeEditor() {
        if case .saveEditor(let approval) = pendingOK?.then {
            approval.editor.saving = false
            pendingOK = nil
        }
        saveApproval = nil
        editor = nil
    }

    func addApp(_ identity: String, to editor: ModeEditor) {
        guard !editor.isDefault, !editor.apps.contains(where: { AppIdentity.same($0, identity) }) else { return }
        editor.apps.append(identity)
        editor.error = nil
    }

    func removeApp(_ identity: String, from editor: ModeEditor) {
        editor.apps.removeAll { AppIdentity.same($0, identity) }
    }

    /// The editor's Confirm: the user agreed to where the mode's model sends now. Saved with the mode.
    func confirmInEditor(_ editor: ModeEditor) {
        guard !editor.pinChanged,
              case .confirm(let choice, _, _) = polishState(polish: true, pin: editor.polishModel, modelName: editor.modelNameToSend,
                                                           pinState: editor.original?.polishModelState, ignoringSwitch: true)
        else { return }
        editor.confirmedTo = choice
    }

    /// Save: the mode as the editor holds it. A model newly picked (or confirmed) at a destination
    /// no polish consent covers asks for its OK first; the save follows Allow.
    func save() {
        guard let editor, !editor.saving else { return }
        editor.error = nil
        if editor.polish, editor.pinChanged || confirmed(editor) || !(editor.original?.polish ?? false),
           case .needsOK(let choice, _, own: true) = polishState(editor, ignoringSwitch: true) {
            saveApproval = SaveApproval(editor: editor, choice: choice, modelName: editor.modelNameToSend)
            editor.consentStep = choice.destination
            return
        }
        sendSave(editor)
    }

    /// The editor's OK step: Allow records polish's OK for `destination` (the one the step showed),
    /// then saves.
    func allowAndSave(_ destination: ConsentModel.Destination) {
        guard let editor, editor.consentStep == destination, let consent else { return }
        editor.consentStep = nil
        guard let approval = saveApproval, approval.choice.destination == destination,
              approval.matches(editor, choices: choices) else {
            saveApproval = nil
            editor.error = Self.saveFailure(.destinationChanged, editor: editor)
            return
        }
        saveApproval = nil
        editor.saving = true
        pendingOK = (consent.allow(forMode: destination), destination, .saveEditor(approval))
    }

    /// The editor's OK step's Cancel: nothing is recorded or saved; the editor stays.
    func cancelConsentStep() {
        saveApproval = nil
        editor?.consentStep = nil
    }

    private func sendSave(_ editor: ModeEditor, replaceUnreadable: Bool = false, approvedTo: ModelChoice? = nil) {
        let original = editor.original
        var save = ModeSave(id: editor.modeID)
        // A new mode names every field; a change only what it changes.
        func changed<T: Equatable>(_ value: T, _ was: T?) -> T? { original == nil || value != was ? value : nil }
        save.name = changed(editor.name, original?.name)
        save.style = editor.style == .other ? nil : changed(editor.style, original?.style)
        save.polish = changed(editor.polish, original?.polish)
        save.removeFillers = changed(editor.removeFillers, original?.removeFillers)
        save.polishPrompt = changed(editor.prompt, original?.polishPrompt)
        if !editor.isDefault {
            let same = original.map { $0.apps.count == editor.apps.count && zip($0.apps, editor.apps).allSatisfy(==) } ?? false
            save.apps = same ? nil : editor.apps
        }
        if editor.pinChanged || original == nil {
            save.polishModel = .some(editor.polishModel)
            save.polishModelName = .some(editor.modelNameToSend)
        }
        if let approvedTo {
            save.confirmTo = approvedTo
        } else if !editor.pinChanged, let shown = editor.confirmedTo, shown.id == editor.polishModel {
            // As the user was shown it: if it sends elsewhere by now, the core refuses it.
            save.confirmTo = shown
        }
        save.takeApps = editor.takeApps || !movingNotes(editor).isEmpty
        // Said for this save only: an app someone else takes later is said again.
        editor.takeApps = false
        save.replaceUnreadable = replaceUnreadable
        let ref = nextRef()
        pendingSaves[ref] = editor
        editor.saving = true
        send(.modesSave(save, ref: ref))
    }

    // MARK: - The rows' actions

    func askDelete(_ id: String) {
        guard !busy else { return }
        deleting = rows.first { $0.id == id && !$0.isDefault }
    }

    /// The delete was confirmed, of `row` (the one the confirmation named): its apps go back to
    /// the default mode.
    func delete(_ row: ModeRow) {
        guard deleting?.id == row.id, !busy else { return }
        deleting = nil
        problem = nil
        busy = true
        let ref = nextRef()
        pendingDelete = (ref, row.name)
        send(.modesDelete(mode: row.id, ref: ref))
    }

    /// The delete confirmation's words: "Slack and Mail go back to Everywhere else."
    static func deleteMessage(_ row: ModeRow) -> String {
        let names = row.apps.map(\.name)
        guard !names.isEmpty else { return "It has no apps. Voice commands can't switch to it any more." }
        return "\(ListFormatter.localizedString(byJoining: names)) \(names.count == 1 ? "goes" : "go") back to \(everywhereElse)."
    }

    /// A row's Confirm… or Allow….
    func askConfirm(_ id: String) {
        guard !busy, let row = rows.first(where: { $0.id == id }) else { return }
        problem = nil
        switch row.polish {
        case .confirm(let choice, let label, _):
            confirming = Confirming(modeID: id, modeName: row.name, choice: choice, label: label, pin: true)
        case .needsOK(let choice, let label, own: true):
            confirming = Confirming(modeID: id, modeName: row.name, choice: choice, label: label, pin: false)
        default:
            confirming = nil
        }
    }

    /// The confirmation's title, message and button.
    static func confirmTitle(_ c: Confirming) -> String {
        "Polish \u{201C}\(c.modeName)\u{201D} with \(c.label)?"
    }

    func confirmMessage(_ c: Confirming) -> String {
        c.asksOK ? consentMessage(c.choice.destination) : ConsentModel.message(.polish, c.choice.destination)
    }

    static func confirmButton(_ c: Confirming) -> String {
        ConsentModel.button(.polish, c.choice.destination)
    }

    /// The confirmation's Allow: polish's OK for where the model sends, if no consent covers it,
    /// then (for a model that moved) the confirm, with the destination the user was shown.
    func confirmAllow(_ c: Confirming) {
        guard confirming == c, !busy else { return }
        confirming = nil
        busy = true
        if c.asksOK, let consent {
            pendingOK = (consent.allow(forMode: c.choice.destination), c.choice.destination, c.pin ? .confirm(c) : .nothing)
        } else if c.pin {
            sendConfirm(c)
        } else {
            busy = false
        }
    }

    private func sendConfirm(_ c: Confirming) {
        var save = ModeSave(id: c.modeID)
        save.confirmTo = c.choice
        let ref = nextRef()
        pendingConfirm = ref
        send(.modesSave(save, ref: ref))
    }

    /// Start over: asks first (it replaces the stored modes, which can't be read, with the default).
    func askStartOver() {
        guard unreadable, !busy else { return }
        confirmingStartOver = true
    }

    /// Start over's confirmation: words for it.
    static let startOverTitle = "Start over?"
    static let startOverMessage = "Your stored modes can't be read. Start over replaces them with one default mode, Everywhere else; the modes that can't be read are gone."

    /// Replaces stored modes the core can't read with the default: only when the user confirmed it.
    func startOver() {
        guard unreadable, confirmingStartOver, !busy else { return }
        confirmingStartOver = false
        busy = true
        problem = nil
        var save = ModeSave(id: Self.builtinDefaultID)
        save.replaceUnreadable = true
        let ref = nextRef()
        pendingStartOver = ref
        send(.modesSave(save, ref: ref))
    }

    /// The default mode's id when the core starts over (ink-pipeline's Mode::builtin_default).
    static let builtinDefaultID = "default"

    // MARK: - Refusals in words

    /// What a refused save says, by its code (never the core's message: that names fields).
    static func saveFailure(_ code: FailureCode?, editor: ModeEditor?) -> String {
        switch code {
        case .nameBlank: return "Give the mode a name."
        case .nameTaken: return "Another mode has a name that sounds the same. Pick another name."
        case .nameIsStyle: return "Formal, Casual and Relaxed name the styles in voice commands. Pick another name."
        case .tooLong:
            let name = editor?.name.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            // Counted as the core counts: Unicode scalars, as Rust's chars.
            if name.unicodeScalars.count > 64 { return "A name can be up to 64 characters." }
            if (editor?.prompt.unicodeScalars.count ?? 0) > promptLimit { return "Polish instructions can be up to 2,000 characters." }
            if (editor?.apps.count ?? 0) > 64 { return "Shorten to 64 apps or fewer." }
            if editor?.adding == true { return "You can have up to 50 modes." }
            return "Something here is too long. Shorten it."
        case .defaultMode: return "\(everywhereElse) is used in every app without a mode of its own, so it can't be given apps."
        case .appTaken: return "An app here is in another mode now. Save again to move it here."
        case .appInvalid: return "One of these apps can't be told apart from others. Remove it, and pick it again."
        case .modeNotFound: return "This mode was deleted meanwhile, so it wasn't saved."
        case .modelUnknown: return "That model isn't available any more. Pick another."
        case .modelNameInvalid: return "A model name can be up to 128 characters, on one line."
        case .destinationChanged: return "Where its model sends changed while you looked. Check it, and confirm again."
        case .listUnreadable: return "Your modes can't be read, so this wasn't saved. Start over replaces them with the default."
        // Codes of other commands (meetings, model downloads): never a mode save's, so the generic line.
        case .meetingRecording, .deleteWindowOver, .notEnoughSpace, .modelInUse, .notDownloading, nil:
            return "Couldn't save the mode. Try again."
        }
    }

    /// What a refused delete says.
    static func deleteFailure(_ code: FailureCode?, name: String) -> String {
        switch code {
        case .defaultMode: "\(everywhereElse) can't be deleted: it is used in every app without a mode of its own."
        case .modeNotFound: "\u{201C}\(name)\u{201D} was already deleted."
        case .listUnreadable: "Your modes can't be read, so \u{201C}\(name)\u{201D} wasn't deleted. Start over replaces them with the default."
        default: "Couldn't delete \u{201C}\(name)\u{201D}. Try again."
        }
    }

    /// What a refused confirm says.
    static func confirmFailure(_ code: FailureCode?) -> String {
        switch code {
        case .destinationChanged: "Where its model sends changed while you looked. Check it, and confirm again."
        case .modeNotFound: "That mode was deleted meanwhile."
        case .modelUnknown: "Its model isn't available any more. Edit the mode to pick another."
        case .listUnreadable: "Your modes can't be read, so nothing was confirmed. Start over replaces them with the default."
        default: "Couldn't confirm its model. Try again."
        }
    }

    /// A save refused after its editor was closed.
    static func lateSaveFailure(_ editor: ModeEditor, _ words: String) -> String {
        let name = editor.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let what = editor.adding ? (name.isEmpty ? "The new mode" : "The new mode \u{201C}\(name)\u{201D}") : "Your change to \u{201C}\(editor.original?.name ?? name)\u{201D}"
        return "\(what) wasn't saved. \(words)"
    }

    static let okFailure = "Couldn't record your OK, so nothing was saved. Try again."
    static let okFailureRow = "Couldn't record your OK. Try again."
    static let startOverFailure = "Couldn't start over. Try again."

    // MARK: - Events

    func apply(_ event: InkEvent) {
        switch event {
        case .modesListed(let listed):
            show(listed)
            if let ref = listed.ref {
                if let sender = pendingSaves.removeValue(forKey: ref) {
                    // Saved: the editor that sent it closes (unless it already did).
                    if editor === sender { editor = nil }
                } else if ref == pendingDelete?.ref {
                    pendingDelete = nil
                    finished()
                } else if ref == pendingConfirm {
                    pendingConfirm = nil
                    finished()
                } else if ref == pendingStartOver {
                    pendingStartOver = nil
                    finished()
                }
            }
        case .commandFailed(let failure) where failure.command == "modes.list":
            // Not known what is stored: nothing is shown, and nothing can be changed. Try again reads
            // again; Start over (which asks first) replaces modes the core can't read, and changes
            // nothing over ones it can.
            failed = true
            unreadable = true
            rows = []
            listed = nil
        case .commandFailed(let failure) where failure.command == "modes.save" || failure.command == "modes.delete":
            refused(failure)
        case .commandFailed(let failure) where failure.command == "consent.allow" && failure.id == pendingOK?.ref:
            okFailed()
        case .consentState(let state) where state.feature == .polish:
            if let pending = pendingOK, state.ref == pending.ref {
                if ConsentModel.Snapshot(state).covers(pending.destination) {
                    pendingOK = nil
                    switch pending.then {
                    case .saveEditor(let approval):
                        guard approval.matches(editor, choices: choices) else {
                            approval.editor.saving = false
                            approval.editor.error = Self.saveFailure(.destinationChanged, editor: approval.editor)
                            break
                        }
                        // Send the destination the user approved, even when replacing the pin.
                        sendSave(approval.editor, approvedTo: approval.choice)
                    case .confirm(let c):
                        sendConfirm(c)
                    case .nothing:
                        // The OK was all the row asked for.
                        finished()
                    }
                } else {
                    // The model sends elsewhere now, or the store refused: nothing was recorded.
                    okFailed()
                }
            }
            // Which model each mode may use now.
            reload()
        case .llmProviders, .engineUnregistered:
            reload()
        case .engineRegistered(let engine) where engine.kind == .llm:
            reload()
        case .settingValue(let value) where value.key == ShellSetting.llmLocalOnly.rawValue:
            reload()
        case .coreStopped:
            // Nothing in flight will be answered: the buttons are back, and nothing waits.
            pendingSaves.values.forEach { $0.saving = false }
            pendingSaves = [:]
            pendingDelete = nil
            pendingConfirm = nil
            pendingStartOver = nil
            if case .saveEditor(let approval) = pendingOK?.then { approval.editor.saving = false }
            pendingOK = nil
            saveApproval = nil
            editor?.consentStep = nil
            busy = false
        case .dictationWarningEvent(let warning) where warning.kind == .polishModelMissing:
            // A take found a mode's model gone or moved: show which.
            reload()
        default:
            break
        }
    }

    /// Reads the modes again after a change elsewhere, once Settings has read them.
    private func reload() {
        guard loaded else { return }
        send(.modesList(ref: nextRef()))
    }

    /// A row's operation has its answer: a stale failure goes, and the buttons are back.
    private func finished() {
        busy = false
        problem = nil
    }

    private func okFailed() {
        guard let pending = pendingOK else { return }
        pendingOK = nil
        switch pending.then {
        case .saveEditor(let approval):
            approval.editor.saving = false
            approval.editor.error = Self.okFailure
        case .confirm, .nothing:
            busy = false
            problem = Self.okFailureRow
        }
    }

    private func refused(_ failure: CommandFailed) {
        guard let id = failure.id else { return }
        if failure.code == .listUnreadable {
            unreadable = true
        }
        if let sender = pendingSaves.removeValue(forKey: id) {
            sender.saving = false
            if failure.code == .appTaken {
                sender.takeApps = true
            }
            if [.destinationChanged, .modelUnknown].contains(failure.code) {
                // What was confirmed is not where the model sends now: Confirm asks again.
                sender.confirmedTo = nil
            }
            let words = Self.saveFailure(failure.code, editor: sender)
            if editor === sender {
                sender.error = words
            } else {
                // Cancelled while the save was on its way: said where the user is now.
                problem = Self.lateSaveFailure(sender, words)
            }
        } else if let pending = pendingDelete, id == pending.ref {
            pendingDelete = nil
            busy = false
            problem = Self.deleteFailure(failure.code, name: pending.name)
        } else if id == pendingConfirm {
            pendingConfirm = nil
            busy = false
            problem = Self.confirmFailure(failure.code)
        } else if id == pendingStartOver {
            pendingStartOver = nil
            busy = false
            problem = Self.startOverFailure
            return
        } else {
            return
        }
        // What the refusal says may have changed: show the modes as stored now.
        if [.modeNotFound, .modelUnknown, .destinationChanged, .appTaken].contains(failure.code) {
            reload()
        }
    }

    private func show(_ listed: ModesListed) {
        self.listed = listed
        // An app not on this Mac may have been installed since: asked again.
        labelCache = labelCache.filter { $0.value.installed }
        loaded = true
        failed = false
        unreadable = false
        choices = listed.polishModels.map(ModelChoice.init)
        settingModel = listed.settingPolishModel
        defaultPrompt = listed.defaultPolishPrompt
        // The default mode last, as "everywhere else": the others are matched first.
        let others = listed.modes.filter { $0.id != listed.defaultId }
        let fallback = listed.modes.filter { $0.id == listed.defaultId }
        rows = (others + fallback).map { mode in
            var traits = [Self.style(mode.style)]
            if mode.removeFillers { traits.append("Clean up speech") }
            var seen = Set<String>()
            let labels = mode.apps
                .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
                .map { label($0) }
                .filter { seen.insert($0.id).inserted }
            let isDefault = mode.id == listed.defaultId
            return ModeRow(
                id: mode.id, name: mode.name, isDefault: isDefault,
                title: isDefault && listed.modes.count > 1 ? Self.everywhereElse : mode.name,
                traits: traits,
                polish: polishState(polish: mode.polish, pin: mode.polishModel, modelName: mode.polishModelName,
                                    pinState: mode.polishModelState),
                apps: labels)
        }
    }
}
