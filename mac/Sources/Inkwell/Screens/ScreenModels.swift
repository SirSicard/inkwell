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
    }

    /// The app became active again.
    func appBecameActive() {
        permissions.appBecameActive()
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
        case "setting.get", "setting.set":
            failed.id == OnboardingModel.settingID || MeetingModel.settingIDs.contains(failed.id ?? "")
        default:
            false
        }
    }
}
