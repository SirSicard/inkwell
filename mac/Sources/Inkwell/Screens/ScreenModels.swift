// The view models of the Live, Owed and Settings screens and the first-run state, fed by the core's
// events after the CoreStore (CoreController.received), and sending their commands through the
// controller. One instance per app, in the window's environment.
import AppKit
import AppleEngines
import Foundation
import InkBridge

/// The first-run state: shown until the user completes or skips it, remembered in the core's store.
@MainActor
@Observable
final class OnboardingModel {
    enum Step: Int, CaseIterable, Sendable {
        case welcome
        case permissions
        /// The speech models: downloaded only when the user presses Download there.
        case models
        case polish
        case ready
    }

    /// nil until the store answers; then whether it was completed.
    private(set) var completed: Bool?
    /// The app is quitting: the sheet is ended so AppKit can quit, and nothing is recorded.
    private(set) var quitting = false
    var step: Step = .welcome

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let log: ScreenLog

    init(send: @escaping SendCommand, log: ScreenLog = .system) {
        self.send = send
        self.log = log
    }

    /// Whether the window shows it.
    var showing: Bool { completed == false && !quitting }

    func load() {
        send(.settingGet(.onboardingDone))
    }

    func next() {
        if let following = Step(rawValue: step.rawValue + 1) {
            step = following
        } else {
            finish()
        }
    }

    func back() {
        if let previous = Step(rawValue: step.rawValue - 1) {
            step = previous
        }
    }

    /// The app is quitting (AppKit does not quit while a window has a sheet): the sheet goes, and
    /// the first run stays not completed, so the next launch shows it.
    func appQuitting() {
        quitting = true
    }

    /// The sheet went away without Start or Skip (Escape): skipped, unless the app is quitting.
    func sheetDismissed() {
        if showing {
            finish()
        }
    }

    /// Done or skipped: not shown again.
    func finish() {
        completed = true
        send(.settingSet(.onboardingDone, "true"))
    }

    /// The id of this model's setting commands (CoreCommand.json gives each setting command one).
    static let settingID = "setting:\(ShellSetting.onboardingDone.rawValue)"

    func apply(_ event: InkEvent) {
        switch event {
        case .settingValue(let value) where value.key == ShellSetting.onboardingDone.rawValue:
            completed = value.value == "true"
        case .commandFailed(let failed) where failed.command == "setting.get" && failed.id == Self.settingID:
            // Not known whether it was completed: show it rather than never show it. Completing it
            // again costs a click; a first run that never appears costs the permissions.
            log.write("setting.get for onboarding.done failed; showing the first run")
            if completed == nil {
                completed = false
            }
        default:
            break
        }
    }
}

/// Settings > Storage: where the library lives, and how much room each part takes.
@MainActor
@Observable
final class StorageModel {
    struct Sizes: Equatable, Sendable {
        /// The library database (transcripts, notes, summaries).
        var library: Int64 = 0
        /// Meeting recordings.
        var recordings: Int64 = 0
        /// Speech models.
        var models: Int64 = 0
    }

    let dataDirectory: URL?
    let modelsDirectory: URL?
    private(set) var sizes: Sizes?
    @ObservationIgnored private var measuring = false

    init(dataDirectory: URL?, modelsDirectory: URL?) {
        self.dataDirectory = dataDirectory
        self.modelsDirectory = modelsDirectory
    }

    /// Measures off the main thread.
    func measure() {
        guard !measuring, let data = dataDirectory else { return }
        measuring = true
        let models = modelsDirectory ?? data.appendingPathComponent("models", isDirectory: true)
        Task.detached(priority: .utility) {
            let sizes = Self.sizes(data: data, models: models)
            await MainActor.run { [weak self] in
                self?.sizes = sizes
                self?.measuring = false
            }
        }
    }

    /// Sums the files under `data`: the library's database files, the models (under `models`,
    /// which may be elsewhere), and everything else as recordings.
    nonisolated static func sizes(data: URL, models: URL) -> Sizes {
        var sizes = Sizes()
        let modelsPath = models.standardizedFileURL.path + "/"
        for (url, size) in files(under: data) {
            let path = url.standardizedFileURL.path
            if path.hasPrefix(modelsPath) {
                continue
            } else if url.lastPathComponent.hasPrefix("library.sqlite") {
                sizes.library += size
            } else if !["inkwell.lock", "inkwell.sock"].contains(url.lastPathComponent) {
                sizes.recordings += size
            }
        }
        sizes.models = files(under: models).reduce(0) { $0 + $1.1 }
        return sizes
    }

    nonisolated private static func files(under root: URL) -> [(URL, Int64)] {
        let keys: [URLResourceKey] = [.isRegularFileKey, .totalFileAllocatedSizeKey, .fileSizeKey]
        guard let walk = FileManager.default.enumerator(at: root, includingPropertiesForKeys: keys) else {
            return []
        }
        var out: [(URL, Int64)] = []
        for case let url as URL in walk {
            guard let values = try? url.resourceValues(forKeys: Set(keys)), values.isRegularFile == true else {
                continue
            }
            out.append((url, Int64(values.totalFileAllocatedSize ?? values.fileSize ?? 0)))
        }
        return out
    }

    /// Opens the library's folder in the Finder.
    func showInFinder() {
        if let dataDirectory {
            NSWorkspace.shared.activateFileViewerSelecting([dataDirectory])
        }
    }
}

@MainActor
@Observable
final class ScreenModels {
    let permissions: PermissionsModel
    let polish: PolishModel
    let catalogue: CatalogueModel
    let modes: ModesModel
    let owed: OwedModel
    let live: LiveModel
    let meetings: MeetingModel
    let onboarding: OnboardingModel
    let storage: StorageModel
    let dictation: DictationModel
    /// Voice edit's consent (the key is the dictation model's).
    let editConsent: ConsentModel
    /// The consent and switch for a meeting's summary and Ask (Settings > AI).
    let meetingsConsent: ConsentModel
    let snippets: SnippetsModel
    let voiceCommands: VoiceCommandsModel
    let importNote: ImportNoteModel

    /// The id of the meetings switch's command (a `command.failed` carries it).
    static let meetingsAISettingID = "setting:\(ShellSetting.meetingsLLM.rawValue)"

    @ObservationIgnored private let send: SendCommand

    init(
        send: @escaping SendCommand,
        calendar: any CalendarAccess = EventKitCalendar(),
        apps: any AppDirectory = WorkspaceApps(),
        callTitles: any CallTitles = EventKitCallTitles(),
        dataDirectory: URL? = nil,
        modelsDirectory: URL? = nil,
        log: ScreenLog = .system
    ) {
        permissions = PermissionsModel(send: send, calendar: calendar)
        polish = PolishModel(send: send)
        catalogue = CatalogueModel(send: send)
        modes = ModesModel(send: send, apps: apps)
        owed = OwedModel(send: send)
        live = LiveModel(send: send)
        meetings = MeetingModel(send: send, titles: callTitles)
        onboarding = OnboardingModel(send: send, log: log)
        storage = StorageModel(dataDirectory: dataDirectory, modelsDirectory: modelsDirectory)
        dictation = DictationModel(send: send)
        editConsent = ConsentModel(feature: .edit, switchSettingID: DictationModel.editKeySettingID, send: send)
        meetingsConsent = ConsentModel(feature: .meetings, switchSettingID: Self.meetingsAISettingID, send: send)
        self.send = send
        dictation.hasLanguageModel = { [polish] in polish.hasWorkingEngine }
        snippets = SnippetsModel(send: send)
        voiceCommands = VoiceCommandsModel(send: send)
        importNote = ImportNoteModel(send: send)
    }

    /// A batch of the core's events, after the CoreStore has applied it.
    func apply(_ batch: [InkEvent]) {
        for event in batch {
            if case .coreReady = event {
                coreReady()
            }
            permissions.apply(event)
            polish.apply(event)
            catalogue.apply(event)
            modes.apply(event)
            owed.apply(event)
            live.apply(event)
            meetings.apply(event)
            onboarding.apply(event)
            dictation.apply(event)
            editConsent.apply(event)
            meetingsConsent.apply(event)
            snippets.apply(event)
            voiceCommands.apply(event)
            importNote.apply(event)
        }
    }

    /// The core started: read what the first screens need. The permission check is the one after
    /// each launch (the Today banner and the sidebar read it too).
    private func coreReady() {
        onboarding.load()
        polish.load()
        meetings.load()
        permissions.refresh()
        catalogue.requery()
        // Reads the switch, then (unless it is off) the core holds the keys; without
        // Accessibility it answers dictation.off, and coming back to the app tries again.
        dictation.load()
        editConsent.load()
        meetingsConsent.load()
    }

    /// The Voice section's edit picker. Off turns voice edit off (the core withdraws its consent in
    /// the same write). A key while voice edit is on and allowed only changes the key; otherwise
    /// the consent step asks first (Allow sends the key with the consent; Cancel changes nothing).
    func chooseEditKey(_ token: String?) {
        guard let token else {
            editConsent.switchedOff()
            dictation.setEditKey(nil)
            return
        }
        if dictation.editKey != nil && editConsent.isAllowedOn {
            dictation.setEditKey(token)
        } else {
            editConsent.ask(from: .settings, key: token)
        }
    }

    // MARK: - Summaries and Ask (Settings > AI)

    /// Their switch reads on only with a consent that covers the model now and a working model.
    var meetingsAIOn: Bool { meetingsConsent.isAllowedOn && polish.hasWorkingEngine }

    /// Whether their switch can be used: a working model, and the core has said where it sends.
    var canToggleMeetingsAI: Bool {
        polish.hasWorkingEngine && meetingsConsent.state != nil && meetingsConsent.destination != nil
    }

    /// The line under their switch. A failure or an unread state is said first, never read as off.
    var meetingsAIStatus: String {
        if meetingsConsent.failure != nil || meetingsConsent.state?.error != nil, let problem = meetingsConsent.problem {
            return problem
        }
        guard polish.hasWorkingEngine else {
            return "No language model is available on this Mac, so meetings get no summary."
        }
        if let problem = meetingsConsent.problem { return problem }
        guard meetingsAIOn, let destination = meetingsConsent.destination else {
            return "Off. Meetings are recorded and transcribed, with no summary, and Ask stays off."
        }
        return destination.isOnDevice
            ? "On, with \(destination.label). Meeting transcripts stay on this Mac."
            : "On. Meeting transcripts go to \(destination.label) for summaries and Ask."
    }

    /// The user switched summaries and Ask. On shows the consent step (nothing is sent until
    /// Allow); off turns them off, and the core withdraws the consent in the same write.
    func setMeetingsAI(_ on: Bool) {
        guard canToggleMeetingsAI else { return }
        if on {
            meetingsConsent.ask(from: .settings)
            return
        }
        meetingsConsent.switchedOff()
        send(.settingSet(.meetingsLLM, "off"))
    }

    /// What a record without a summary says while summaries are off (nil: on, or not read yet).
    var summaryOffNote: String? {
        guard meetingsConsent.state != nil, !meetingsConsent.isAllowedOn else { return nil }
        return "Summaries are off until you allow them in Settings > AI. Meetings are still recorded and transcribed."
    }

    /// The app became active again.
    func appBecameActive() {
        permissions.appBecameActive()
        dictation.appBecameActive()
    }

    /// The core is about to stop: hand it what the screens hold unsaved.
    func flushBeforeStop() {
        live.notesLeft()
    }

    /// Whether a screen shows this failure itself (the rest the controller logs).
    func handles(_ failed: CommandFailed) -> Bool {
        switch failed.command {
        case "permissions.check", "models.list", "modes.list", "commitment.set_done",
             "commitment.not_yet", "note.add", "note.update", "note.delete",
             "meeting.start", "meeting.stop", "meeting.dismiss", "meeting.ask":
            true
        case "model.update":
            // The download's row says it failed, and why (the first run and Settings > Models).
            true
        case "setting.get":
            failed.id == OnboardingModel.settingID || failed.id == PolishModel.settingID
                || MeetingModel.settingIDs.contains(failed.id ?? "") || dictation.handles(failed)
        case "setting.set":
            // Onboarding's is not shown (the first run shows again next launch), so it is logged.
            failed.id == PolishModel.settingID || MeetingModel.settingIDs.contains(failed.id ?? "")
                || failed.id == Self.meetingsAISettingID || dictation.handles(failed)
        case "dictation.enable", "dictation.disable":
            dictation.handles(failed)
        case "engine.route":
            // Settings > Models says so on the job's line.
            CatalogueModel.routeJob(failed) != nil
        case "consent.get", "consent.allow":
            // Shown under the Polish or the summaries toggle, or in the Voice section for voice edit.
            true
        // Settings > Snippets and Voice commands say so. The key note that could not be read is
        // not shown (there is nothing to say then); it is logged.
        case "snippets.list", "snippets.save", "voice_commands.list", "voice_commands.save":
            true
        default:
            false
        }
    }
}
