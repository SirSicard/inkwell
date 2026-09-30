// Settings > Models: which engine does each job now, and how accurate it measured. Read-only.
//
// What serves a job is the router's answer to engine.route, and engine.routed answers only that
// command: nothing announces a change. So the screen asks again whenever the answer may have
// changed: when it appears, after a model update finishes (a model installed or replaced), and
// when an engine the shell registers comes or goes (Parakeet loaded, Apple Intelligence turned
// on or off).
import InkBridge
import Observation

@MainActor
@Observable
final class CatalogueModel {
    /// The jobs the screen lists, in order.
    static let jobs: [Job] = [.dictationFinal, .meetingFinal, .livePartials]

    /// The catalogue's models for this OS.
    private(set) var models: [CatalogueEntry] = []
    /// The last models.list failed: the list is not known, which is not the same as empty.
    private(set) var failed = false

    static let failedText = "The model list could not be read."
    /// Engines the shell registered, with their measured rates.
    private(set) var shellEngines: [String: [JobScore]] = [:]
    /// What serves each job, as last answered; a job asked about and unfilled maps to nil.
    private(set) var serving: [Job: EngineRouted] = [:]
    /// Jobs whose last engine.route failed: not known, which is not "nothing installed".
    private(set) var routeFailed: Set<Job> = []

    nonisolated static let routeFailedText = "Couldn't check which model does this"
    /// Times the screen asked again (tests and diagnostics).
    @ObservationIgnored private(set) var requeries = 0

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
        /// The last question about it failed (engine.route).
        var failed = false

        var id: Job { job }

        /// What the line says in place of an engine: the engine, else why not, or still asking.
        var engineText: String {
            engine ?? (failed ? CatalogueModel.routeFailedText : known ? "Nothing installed yet" : "Checking…")
        }

        /// The measured accuracy, as the screen shows it.
        var accuracy: String? {
            guard let wer else { return nil }
            let right = max(0, 100 - wer)
            return String(format: "%.1f %% of words right (%.1f %% word error rate)", right, wer)
        }
    }

    func line(_ job: Job) -> Line {
        if routeFailed.contains(job) {
            return Line(job: job, engine: nil, wer: nil, known: false, failed: true)
        }
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
        case "fluidaudio-parakeet-tdt-0.6b-v3", "fluidaudio-parakeet-tdt-0.6b-v3-offline": "Parakeet TDT v3"
        case "apple-foundation-models": "Apple Foundation Models"
        case "nemotron-3-diarization-q8": "Nemotron-3-Diarization"
        case "silero-vad-v6": "Silero VAD"
        default: id
        }
    }

    /// The job an engine.route failure is about, from its id; nil when it names none of the
    /// screen's jobs (the controller logs that one).
    static func routeJob(_ failed: CommandFailed) -> Job? {
        guard failed.command == "engine.route" else { return nil }
        return jobs.first { CoreCommand.engineRoute($0).commandID == failed.id }
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .modelsListed(let listed):
            models = listed.models
            failed = false
        case .commandFailed(let failure) where failure.command == "models.list":
            failed = true
        case .engineRouted(let routed):
            serving[routed.job] = routed
            routeFailed.remove(routed.job)
        case .commandFailed(let failure):
            if let job = Self.routeJob(failure) { routeFailed.insert(job) }
        case .engineRegistered(let engine):
            shellEngines[engine.id] = engine.jobs
            requery()
        case .engineUnregistered(let engine):
            shellEngines[engine.id] = nil
            requery()
        case .modelUpdateFinished:
            requery()
        case .coreStopped:
            shellEngines = [:]
            serving = [:]
            routeFailed = []
        default:
            break
        }
    }
}
