// Settings > Models and the first run's Models step: which engine does each job now, how accurate
// it measured, and the catalogue's models, each downloaded only when the user presses Download.
//
// What serves a job is the router's answer to engine.route, and engine.routed answers only that
// command: nothing announces a change. So the screen asks again whenever the answer may have
// changed: when it appears, after a model update finishes (a model installed or replaced), and
// when an engine the shell registers comes or goes (Parakeet loaded, Apple Intelligence turned
// on or off).
//
// Downloads start only from a press (`download`), never on their own: not at launch, not after a
// failure, not to finish one a quit interrupted. The models pressed for install one at a time, in
// the order pressed (`model.update` with the model as its own next); the next is sent once the one
// before has ended, with model.update_finished or a command.failed carrying its id. Only the model
// being installed moves a bar: progress for another model, or for a replacement (one model updated
// to another), is not this screen's. A failure stays, in the core's words, until the user tries
// again.
import InkBridge
import Observation

@MainActor
@Observable
final class CatalogueModel {
    /// The jobs the screen lists, in order.
    static let jobs: [Job] = [.dictationFinal, .meetingFinal, .livePartials]

    /// The catalogue's models for this OS.
    private(set) var models: [CatalogueEntry] = []
    /// Whether models.list has been answered: until then the list is not known yet.
    private(set) var listed = false
    /// The last models.list failed: the list is not known, which is not the same as empty.
    private(set) var failed = false

    static let failedText = "The model list could not be read."
    /// Engines the shell registered, with their measured rates.
    private(set) var shellEngines: [String: [JobScore]] = [:]
    /// What serves each job, as last answered; a job asked about and unfilled maps to nil.
    private(set) var serving: [Job: EngineRouted] = [:]
    /// Times the screen asked again (tests and diagnostics).
    @ObservationIgnored private(set) var requeries = 0

    /// The model downloading now, and how far it has got (nil until its first progress).
    private(set) var installing: String?
    private(set) var progress: DownloadProgress?
    /// Models the user asked for, waiting their turn, in order.
    private(set) var waiting: [String] = []
    /// Why each model's last download failed, in the core's words.
    private(set) var failures: [String: String] = [:]
    /// Every model the user asked for since launch: the first run keeps listing them once they
    /// are in, as downloaded.
    private(set) var asked: Set<String> = []
    /// The id the install's command carries: a command.failed with it is that install's failure.
    @ObservationIgnored private var installRef: String?
    @ObservationIgnored private var installs = 0

    @ObservationIgnored private let send: SendCommand

    init(send: @escaping SendCommand) {
        self.send = send
    }

    /// Asks what serves every job, and for the catalogue.
    func requery() {
        requeries += 1
        send(.modelsList)
        for job in Self.jobs {
            send(.engineRoute(job))
        }
    }

    /// One job's line.
    struct Line: Equatable, Identifiable {
        let job: Job
        /// The engine's name; nil when nothing fills the job.
        let engine: String?
        /// Its measured word error rate on this job, percent.
        let wer: Double?
        /// Whether the answer has arrived.
        let known: Bool

        var id: Job { job }

        /// The measured accuracy, as the screen shows it.
        var accuracy: String? {
            guard let wer else { return nil }
            let right = max(0, 100 - wer)
            return String(format: "%.1f %% of words right (%.1f %% word error rate)", right, wer)
        }
    }

    func line(_ job: Job) -> Line {
        guard let routed = serving[job] else {
            return Line(job: job, engine: nil, wer: nil, known: false)
        }
        guard let id = routed.id else {
            return Line(job: job, engine: nil, wer: nil, known: true)
        }
        let scores = routed.source == .shell
            ? shellEngines[id]
            : models.first { $0.id == id }?.jobs
        return Line(job: job, engine: Self.name(id), wer: scores?.first { $0.job == job }?.wer, known: true)
    }

    /// A job's name.
    static func title(_ job: Job) -> String {
        switch job {
        case .dictationFinal: "Dictation"
        case .meetingFinal: "Meeting transcript"
        case .livePartials: "Live words"
        case .diarization: "Who spoke"
        case .voiceActivity: "Voice detection"
        }
    }

    /// An engine's name. Ids this build does not know are shown as they are: they name a model,
    /// not an app.
    static func name(_ id: String) -> String {
        switch id {
        case "qwen3-asr-1.7b-q8": "Qwen3-ASR 1.7B"
        case "fluidaudio-parakeet-tdt-0.6b-v3", "fluidaudio-parakeet-tdt-0.6b-v3-offline", "parakeet-tdt-0.6b-v3-coreml":
            "Parakeet TDT v3"
        case "apple-foundation-models": "Apple Foundation Models"
        case "nemotron-3-diarization-q8": "Nemotron-3-Diarization"
        case "silero-vad-v6-16k": "Silero VAD"
        default: id
        }
    }

    // MARK: - Downloads

    /// A download's progress: bytes on disk so far and the model's size.
    struct DownloadProgress: Equatable {
        let done: Int64
        let total: Int64

        /// From 0 to 1. The bytes can go down: a file whose server ignored a resume starts over.
        var fraction: Double {
            total > 0 ? min(1, max(0, Double(done) / Double(total))) : 0
        }
    }

    /// What a model's row shows.
    enum Download: Equatable {
        /// On this Mac.
        case installed
        /// Not on this Mac, and not asked for.
        case notInstalled
        /// Asked for, waiting for the download before it.
        case waiting
        /// Downloading: nil until its first progress (the core may still be finishing another
        /// command).
        case downloading(DownloadProgress?)
        /// Its last download failed: why, in the core's words, until the user tries again.
        case failed(String)
    }

    func download(of model: CatalogueEntry) -> Download {
        if model.id == installing { return .downloading(progress) }
        if waiting.contains(model.id) { return .waiting }
        if model.installed { return .installed }
        if let failure = failures[model.id] { return .failed(failure) }
        return .notInstalled
    }

    /// The user pressed Download (the first run's, a row's in Settings, or Retry): each model is
    /// queued once, behind any download under way. Nothing else starts a download.
    func download(_ ids: [String]) {
        for id in ids where id != installing && !waiting.contains(id) {
            failures[id] = nil
            asked.insert(id)
            waiting.append(id)
        }
        installNext()
    }

    /// What the first run lists: the models not on this Mac, and those asked for since launch (so
    /// one that finished stays listed, as downloaded). Smallest first, the order Download fetches
    /// them in: the small ones (voice detection, Parakeet's live words and finals) make dictation
    /// work long before Qwen3-ASR's gigabytes are in.
    var firstRunModels: [CatalogueEntry] {
        models.filter { !$0.installed || asked.contains($0.id) }.sorted { $0.sizeBytes < $1.sizeBytes }
    }

    /// The first run's Download: every model it lists that is not here and not asked for yet.
    func downloadFirstRunModels() {
        download(firstRunModels.filter { download(of: $0) == .notInstalled }.map(\.id))
    }

    /// Where a model's files come from: the host of every URL its row names in the core's registry
    /// (ink-engines). nil for a model this build does not know.
    static func source(_ id: String) -> String? {
        switch id {
        case "qwen3-asr-1.7b-q8", "parakeet-tdt-0.6b-v3-coreml", "nemotron-3-diarization-q8": "huggingface.co"
        case "silero-vad-v6-16k": "github.com"
        default: nil
        }
    }

    /// The hosts `models` come from, as a sentence names them, the one serving the most first:
    /// "huggingface.co", or "huggingface.co and github.com".
    static func sources(_ models: [CatalogueEntry]) -> String {
        var hosts: [String] = []
        for model in models.sorted(by: { $0.sizeBytes > $1.sizeBytes }) {
            if let host = source(model.id), !hosts.contains(host) {
                hosts.append(host)
            }
        }
        return hosts.joined(separator: " and ")
    }

    private func installNext() {
        guard installing == nil, !waiting.isEmpty else { return }
        let id = waiting.removeFirst()
        installs += 1
        let ref = "model.update:\(installs)"
        (installing, installRef, progress) = (id, ref, nil)
        send(.modelInstall(id, ref: ref))
    }

    /// The install under way ended: failed with `failure`, or done. The next one starts.
    private func ended(_ id: String, failure: String?) {
        failures[id] = failure
        (installing, installRef, progress) = (nil, nil, nil)
        installNext()
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .modelsListed(let answer):
            models = answer.models
            listed = true
            failed = false
        case .commandFailed(let failure) where failure.command == "models.list":
            failed = true
        case .commandFailed(let failure) where failure.command == "model.update":
            // Refused before it started (another update held the model, say): that download's
            // failure, matched by its id.
            if let id = installing, failure.id == installRef {
                ended(id, failure: failure.message)
            }
        case .engineRouted(let routed):
            serving[routed.job] = routed
        case .engineRegistered(let engine):
            shellEngines[engine.id] = engine.jobs
            requery()
        case .engineUnregistered(let engine):
            shellEngines[engine.id] = nil
            requery()
        case .modelUpdateProgress(let update):
            if update.id == update.next, update.next == installing {
                progress = DownloadProgress(done: update.doneBytes, total: update.totalBytes)
            }
        case .modelUpdateFinished(let update):
            requery()
            if update.id == update.next, update.next == installing {
                ended(update.next, failure: update.ok ? nil : update.message ?? "the download did not finish")
            }
        case .coreStopped:
            shellEngines = [:]
            serving = [:]
            (installing, installRef, progress, waiting) = (nil, nil, nil, [])
        default:
            break
        }
    }
}
