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
        /// Only while Inkwell 0.2's data is offered (`offersImport`).
        case importData
        /// Light, dark or the system's, and the dot colours.
        case appearance
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

    /// Whether the import step is shown: Inkwell 0.2's data is offered (ScreenModels wires it).
    @ObservationIgnored var offersImport: @MainActor () -> Bool = { false }

    /// The steps shown, in order.
    var steps: [Step] { Step.allCases.filter { $0 != .importData || offersImport() } }

    func load() {
        send(.settingGet(.onboardingDone))
    }

    func next() {
        if let following = steps.first(where: { $0.rawValue > step.rawValue }) {
            step = following
        } else {
            finish()
        }
    }

    func back() {
        if let previous = steps.last(where: { $0.rawValue < step.rawValue }) {
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
    /// A measure was asked while one ran: that one may have walked past what changed, so another
    /// follows it.
    @ObservationIgnored private var again = false

    init(dataDirectory: URL?, modelsDirectory: URL?) {
        self.dataDirectory = dataDirectory
        self.modelsDirectory = modelsDirectory
    }

    /// Measures off the main thread.
    func measure() {
        guard let data = dataDirectory else { return }
        if measuring {
            again = true
            return
        }
        measuring = true
        let models = modelsDirectory ?? data.appendingPathComponent("models", isDirectory: true)
        Task.detached(priority: .utility) {
            let sizes = Self.sizes(data: data, models: models)
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.sizes = sizes
                self.measuring = false
                if self.again {
                    self.again = false
                    self.measure()
                }
            }
        }
    }

    /// Measures again when a model's files, or a record's recording, have changed (a model
    /// installed, a record deleted): Settings measures as it appears, and a
    /// download it started finishes while it is still showing (it once read "Models 0 bytes" over
    /// 2.9 GB of installed models). Only once Settings has measured: nobody reads the sizes before.
    func apply(_ event: InkEvent) {
        guard sizes != nil || measuring else { return }
        switch event {
        case .modelUpdateFinished, .recordDeleted:
            // A failed update too: it may have removed what it had downloaded. A deleted record
            // took its recording with it.
            measure()
        default:
            break
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
    /// "Record a shortcut…" for the dictation key and the edit key.
    let shortcuts: ShortcutRecorderModel
    /// Inkwell 0.2's data: the first run's step and a row in Settings > General.
    let import02: Import02Model
    /// The theme: the appearance settings and what they resolve to.
    let theme: GlowTheme
    /// Settings > AI: the language model you bring, and local-only mode.
    let cloud: CloudModel
    /// The Stats screen, milestones, and Settings > Stats.
    let stats: StatsModel

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
        theme = GlowTheme(send: send)
        cloud = CloudModel(send: send)
        stats = StatsModel(send: send)
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
        dictation.speechModels = { [catalogue] in catalogue.speech }
        snippets = SnippetsModel(send: send)
        voiceCommands = VoiceCommandsModel(send: send)
        importNote = ImportNoteModel(send: send)
        import02 = Import02Model(send: send, log: log)
        // A recorded edit key is chosen as a picked one is: consent first when voice edit is not on.
        // Weak: the recorder is the screens' own, and must not keep them alive.
        shortcuts = ShortcutRecorderModel(send: send, dictation: dictation, saveEditKey: { _ in })
        shortcuts.saveEditKey = { [weak self] token in self?.chooseEditKey(token) }
        onboarding.offersImport = { [import02] in import02.offered }
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
            theme.apply(event)
            cloud.apply(event)
            import02.apply(event)
            storage.apply(event)
            stats.apply(event)
            if onboarding.showing {
                // The first run offers its import step only when there is something to import.
                import02.checkOnce()
            }
            if case .importFinished = event {
                // What became of 0.2's key, and the key it set.
                importNote.load()
                send(.settingGet(.dictationKey))
                // The lists it brought, which Settings may show already: an edit to the old list
                // would save it over the import's (the user's own list wins in the core).
                snippets.load()
                voiceCommands.load()
                modes.load()
            }
            dictation.apply(event)
            shortcuts.apply(event)
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
        theme.load()
        // Whether a language model of the user's own can polish (PolishModel reads the answer).
        cloud.load()
        onboarding.load()
        // The sidebar's overdue count.
        owed.load()
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

    /// The Dictation section's edit picker. Off turns voice edit off (the core withdraws its consent in
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

    /// A button on the Drop. The speech models' button brings the main window to Today (`show`),
    /// where the download states its size and hosts; the note stays up for the rest of its time.
    /// The rest are the meeting commands'.
    func performDropAction(_ action: DropText.Action, show: (Route) -> Void) {
        switch action {
        case .showSpeechModels:
            show(.today)
        case .record, .dismiss, .allowSystemAudio: meetings.perform(action, permissions: permissions)
        }
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
        case "stats.get":
            // The Stats screen says it couldn't count. (A milestone check that failed celebrates
            // nothing until the next one; no screen shows it, so it is logged.)
            stats.handles(failed)
        case "setting.get":
            stats.handles(failed) || failed.id == OnboardingModel.settingID || failed.id == PolishModel.settingID
                || MeetingModel.settingIDs.contains(failed.id ?? "") || dictation.handles(failed)
                || GlowTheme.settingIDs.contains(failed.id ?? "") || CloudModel.handles(failed)
        case "setting.set":
            // Onboarding's is not shown (the first run shows again next launch), so it is logged.
            stats.handles(failed) || failed.id == PolishModel.settingID || MeetingModel.settingIDs.contains(failed.id ?? "")
                || failed.id == Self.meetingsAISettingID || dictation.handles(failed)
                || GlowTheme.settingIDs.contains(failed.id ?? "") || CloudModel.handles(failed)
        case "dictation.enable", "dictation.disable":
            dictation.handles(failed)
        case "hotkey.check":
            // Said under the key's row.
            shortcuts.handles(failed)
        case "engine.route":
            // Settings > Models says so on the job's line.
            CatalogueModel.routeJob(failed) != nil
        case "llm.providers", "llm.key.save", "llm.key.delete", "llm.choose", "llm.test":
            // Said under Settings > AI's language model.
            CloudModel.handles(failed)
        case "consent.get", "consent.allow":
            // Shown under the Polish or the summaries toggle, or in the Dictation section for voice edit.
            true
        // Settings > Snippets and Voice commands say so. The key note that could not be read is
        // not shown (there is nothing to say then); it is logged.
        case "snippets.list", "snippets.save", "voice_commands.list", "voice_commands.save":
            true
        // The import says so where it is offered; a check that failed is logged by the model.
        case "import.check", "import.run":
            true
        default:
            false
        }
    }
}
