// Whether this Mac has a speech model: something that writes down what is said, for dictation and
// for the meeting transcript. Without one, a hold types nothing and a meeting is recorded but not
// transcribed, and the screens say so (Today's hero, the Drop after a hold, Live) instead of
// "Hold fn to dictate", "The microphone is silent" and "Waiting for someone to speak".
//
// What is known comes from the router (engine.routed, which CatalogueModel asks for at launch,
// after every install and whenever an engine comes or goes) and the catalogue's list. A job reads
// as missing only when both agree: the router has nothing for it, and no model on this Mac could
// fill it. Parakeet is the exception that rule is for: its files are on disk before the shell has
// loaded and registered it (at launch, and after its download), and until then the router has
// nothing, which is not "no model". Anything not answered, or a question that failed, makes no
// claim.
import InkBridge

struct SpeechModels: Equatable, Sendable {
    enum Availability: Equatable, Sendable {
        /// An engine does the job.
        case served
        /// Nothing installed can do it.
        case missing
        /// Not answered yet, the question failed, or a model on disk is still loading.
        case unknown
    }

    /// Dictation's finals.
    var dictation: Availability
    /// The meeting transcript.
    var meetings: Availability
    /// The recommended set is downloading or waiting its turn.
    var downloading = false

    static let unknown = SpeechModels(dictation: .unknown, meetings: .unknown)

    /// Whether Today offers dictation's key: not when nothing could type what it hears.
    var offersDictation: Bool { dictation != .missing }

    /// Today's line under the greeting; nil when there is nothing to say.
    static func todayLine(_ speech: SpeechModels) -> String? {
        switch (speech.dictation, speech.meetings) {
        case (.missing, .missing): "No speech model yet, so nothing you say can be written down."
        case (.missing, _): "No speech model for dictation yet."
        case (_, .missing): "No speech model for meetings yet, so they are recorded but not transcribed."
        default: nil
        }
    }

    /// Live's line under its header during a meeting nothing can transcribe. The core records the
    /// audio whatever is installed, and its final pass, failing for want of an engine, keeps the
    /// (empty) live transcript and leaves the audio with the record.
    static func liveLine(_ speech: SpeechModels) -> String? {
        speech.meetings == .missing
            ? "No speech model is installed, so this meeting can't be transcribed. Its audio is recorded and kept with the record."
            : nil
    }

    /// What Live's ledger says while nothing has been said.
    static func ledgerEmptyLine(_ speech: SpeechModels) -> String {
        speech.meetings == .missing ? "Nothing can be transcribed without a speech model." : "Waiting for someone to speak."
    }

    /// The Drop's note after a hold that nothing could transcribe: with a button to Today's
    /// download, unless it is already on its way.
    static func dropNote(downloading: Bool) -> DropText {
        downloading
            ? DropText(title: "The speech model is downloading", detail: "Dictation works once it is in")
            : DropText(
                title: "No speech model yet", detail: "Nothing can be typed until one is installed", tone: .alert,
                actions: [.showSpeechModels])
    }
}

/// Where the recommended set's download stands, for Today.
enum SpeechDownload: Equatable, Sendable {
    case notStarted
    /// Downloading or waiting its turn; how far the set has got, when known.
    case downloading(percent: Int?)
    /// A model of the set failed, in the core's words; Download tries again.
    case failed(String)
}

extension CatalogueModel {
    /// What writes speech down on this Mac, as far as is known.
    var speech: SpeechModels {
        SpeechModels(
            dictation: availability(.dictationFinal), meetings: availability(.meetingFinal),
            downloading: Self.recommended.contains { $0 == installing || waiting.contains($0) })
    }

    private func availability(_ job: Job) -> SpeechModels.Availability {
        guard !routeFailed.contains(job), let routed = serving[job] else { return .unknown }
        if routed.id != nil { return .served }
        guard listed, !failed else { return .unknown }
        // A model on disk the router has not taken: a shell engine (no rates of its own until it
        // registers) still loading, or one that fills this job.
        let couldFill = models.contains { model in
            model.installed && (model.jobs.isEmpty || model.jobs.contains { $0.job == job })
        }
        return couldFill ? .unknown : .missing
    }

    /// The recommended set's size in MB, when the list has it.
    var recommendedMB: Int? {
        let sizes = Self.recommended.compactMap { id in models.first { $0.id == id }?.sizeBytes }
        guard sizes.count == Self.recommended.count else { return nil }
        return Int((Double(sizes.reduce(0, +)) / 1_000_000).rounded())
    }

    /// Where the set's download stands.
    var speechDownload: SpeechDownload {
        if Self.recommended.contains(where: { $0 == installing || waiting.contains($0) }) {
            let entries = Self.recommended.compactMap { id in models.first { $0.id == id } }
            let total = entries.reduce(Int64(0)) { $0 + $1.sizeBytes }
            guard let progress, entries.count == Self.recommended.count, total > 0 else {
                return .downloading(percent: nil)
            }
            let done = entries.reduce(Int64(0)) { sum, entry in
                if entry.id == installing { return sum + min(progress.done, entry.sizeBytes) }
                return sum + (isIn(entry.id) ? entry.sizeBytes : 0)
            }
            return .downloading(percent: Int((Double(done) / Double(total) * 100).rounded()))
        }
        if let failure = Self.recommended.lazy.compactMap({ self.failures[$0] }).first {
            return .failed(failure)
        }
        return .notStarted
    }
}
