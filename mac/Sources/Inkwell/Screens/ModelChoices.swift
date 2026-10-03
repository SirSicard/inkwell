// The first run's Speech models step offers choices by what each download does for the user, not a
// list of models: the set every job needs (always included), fewer mistakes (Qwen3-ASR) and
// telling the far end's people apart (the diarizer). One Download fetches what is ticked and
// carries its total. Under each choice its models are still named with their licence, size and
// host: a download happens only where its size and source show.
//
// The state of a choice comes from its models' (CatalogueModel.download(of:)), so installed,
// downloading and failed read as Settings > Models reads them; nothing here starts a download
// except the press (`download(choices:)`, `retry`).
import Foundation
import InkBridge

extension CatalogueModel {
    /// A choice in the first run's Speech models step.
    enum Choice: String, CaseIterable, Identifiable, Sendable {
        /// Voice detection and Parakeet: dictation, the live words and meeting transcripts.
        case transcripts
        /// Qwen3-ASR, for the dictation and meeting finals.
        case accuracy
        /// The diarizer, for the far end's people.
        case speakers

        var id: String { rawValue }

        /// The models it downloads.
        var models: [String] {
            switch self {
            case .transcripts: CatalogueModel.recommended
            case .accuracy: ["qwen3-asr-1.7b-q8"]
            case .speakers: ["nemotron-3-diarization-q8"]
            }
        }

        /// Every job needs the set: it is always included, never a box to untick.
        var isRequired: Bool { self == .transcripts }

        var title: String {
            switch self {
            case .transcripts: "Dictation, live words and meeting transcripts"
            case .accuracy: "Fewer mistakes"
            case .speakers: "Tell the people on the call apart"
            }
        }

        /// What it adds over the set, in the user's terms; nil for the set. The accuracy claim is
        /// checked against the measured rates (ScreensTests).
        var detail: String? {
            switch self {
            case .transcripts: nil
            case .accuracy: "About a third fewer wrong words in dictation and meetings."
            case .speakers: "Speaker 1, Speaker 2 instead of \u{201C}Them\u{201D}."
            }
        }
    }

    /// Where a choice stands, from its models'.
    enum ChoiceState: Equatable {
        /// Every model it names is on this Mac.
        case installed
        /// Something of it is still to download, and not asked for.
        case available
        /// Asked for, waiting for the download before it.
        case waiting
        /// One of its models is downloading: the choice's bytes so far (a finished model counts
        /// whole) and its size; nil until the first progress.
        case downloading(DownloadProgress?)
        /// A model of it failed, in the core's words, until the user tries again.
        case failed(String)
    }

    /// The choices the catalogue can serve (every model each names is listed), in order.
    var choices: [Choice] {
        guard listed else { return [] }
        return Choice.allCases.filter { choice in choice.models.allSatisfy { id in models.contains { $0.id == id } } }
    }

    /// A choice's models as the catalogue lists them, smallest first (the order they download in).
    func entries(_ choice: Choice) -> [CatalogueEntry] {
        models.filter { choice.models.contains($0.id) }.sorted { $0.sizeBytes < $1.sizeBytes }
    }

    /// A choice's size: all of its models, on this Mac or not.
    func size(of choice: Choice) -> Int64 {
        entries(choice).reduce(0) { $0 + $1.sizeBytes }
    }

    /// The size as the step states it: the set's own, an extra's as what it adds ("+2.5 GB").
    func sizeLabel(_ choice: Choice) -> String {
        let size = Self.roundedSize(size(of: choice))
        return choice.isRequired ? size : "+\(size)"
    }

    func state(of choice: Choice) -> ChoiceState {
        let entries = entries(choice)
        if let installing, let current = entries.first(where: { $0.id == installing }) {
            guard let progress else { return .downloading(nil) }
            let done = entries.reduce(Int64(0)) { sum, entry in
                if entry.id == current.id { return sum + min(progress.done, entry.sizeBytes) }
                return sum + (isOnThisMac(entry.id) ? entry.sizeBytes : 0)
            }
            return .downloading(DownloadProgress(done: done, total: size(of: choice)))
        }
        if entries.contains(where: { waiting.contains($0.id) }) { return .waiting }
        if entries.allSatisfy({ isOnThisMac($0.id) }) { return .installed }
        if let failure = entries.lazy.compactMap({ self.failures[$0.id] }).first { return .failed(failure) }
        return .available
    }

    /// The models of `choice` still to fetch: not on this Mac, not downloading or waiting. One
    /// whose last download failed is fetched again.
    private func missing(_ choice: Choice) -> [CatalogueEntry] {
        entries(choice).filter { !isOnThisMac($0.id) && $0.id != installing && !waiting.contains($0.id) }
    }

    /// The set (always) and the ticked extras, the set first, then the extras smallest first.
    private func pressed(_ ticked: Set<Choice>) -> [Choice] {
        let extras = choices.filter { !$0.isRequired && ticked.contains($0) }.sorted { size(of: $0) < size(of: $1) }
        return choices.filter(\.isRequired) + extras
    }

    /// What the Download would fetch for `ticked`, in bytes: what it carries as its total.
    func bytesToDownload(_ ticked: Set<Choice>) -> Int64 {
        pressed(ticked).flatMap(missing).reduce(0) { $0 + $1.sizeBytes }
    }

    /// The hosts the Download would fetch from, for `ticked`.
    func downloadSources(_ ticked: Set<Choice>) -> String {
        Self.sources(pressed(ticked).flatMap(missing))
    }

    /// The step's Download: the set first, so dictation works soonest, then each ticked extra
    /// smallest first, queued behind any download under way (`download` queues each once).
    func download(choices ticked: Set<Choice>) {
        download(pressed(ticked).flatMap(missing).map(\.id))
    }

    /// A failed choice's Retry: its models still to fetch.
    func retry(_ choice: Choice) {
        download(missing(choice).map(\.id))
    }

    /// A model as a choice names it: its name, licence, size (rounded as the step's other sizes
    /// are, so one row never mixes "2.5 GB" with "2,52 GB") and host.
    static func facts(_ entry: CatalogueEntry) -> String {
        var parts = [name(entry.id), entry.licence, roundedSize(entry.sizeBytes)]
        if let source = source(entry.id) {
            parts.append("from \(source)")
        }
        return parts.joined(separator: " · ")
    }

    /// A size as the step rounds it, in decimal units as Today's "484 MB" is: whole megabytes
    /// under a gigabyte, one decimal above. A point, never the locale's comma, so the button reads
    /// as the Windows app's does.
    static func roundedSize(_ bytes: Int64) -> String {
        let mb = Double(bytes) / 1_000_000
        // Compared once rounded, so 999.6 MB reads "1.0 GB", never "1000 MB".
        return mb.rounded() < 1000 ? "\(Int(mb.rounded())) MB" : String(format: "%.1f GB", mb / 1000)
    }

    /// The Download button's title, with the total it fetches.
    static func downloadTitle(_ bytes: Int64) -> String {
        "Download \(roundedSize(bytes))"
    }
}
