// Inkwell 0.2's data on this Mac: looked for when the first run shows and each time Settings
// opens, and imported from the first run's step or Settings > Voice. The core knows where 0.2 kept
// its data; the shell asks (import.check, import.run) and says what came back in plain words.
// After an import, ScreenModels reads the key note and the dictation key again, and the Library
// lists again (import.finished).
import InkBridge
import SwiftUI

@MainActor
@Observable
final class Import02Model {
    /// What the last check found; nil until one answers.
    private(set) var found: ImportChecked?
    /// An import is running.
    private(set) var running = false
    /// What this session's import brought.
    private(set) var imported: ImportCounts?
    /// Why the last import failed, in words.
    private(set) var failure: String?
    /// The last check failed: whether there is anything to import is not known.
    private(set) var checkFailed = false

    /// Whether this app looks for 0.2's data at all: not while the library is moved (INK_DATA_DIR:
    /// development, tests and scripts that must not touch the user's data). CoreController sets it.
    @ObservationIgnored var looks = true

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let log: ScreenLog
    @ObservationIgnored private var asked = false

    init(send: @escaping SendCommand, log: ScreenLog = .system) {
        self.send = send
        self.log = log
    }

    /// Looks for 0.2's data (Settings, each time it opens). Not while an import runs.
    func check() {
        guard looks, !running else { return }
        asked = true
        send(.importCheck)
    }

    /// Looks once (the first run).
    func checkOnce() {
        if !asked {
            check()
        }
    }

    /// Imports (Import). Once per session: after it, there is nothing left to bring over.
    func run() {
        guard looks, !running, imported == nil else { return }
        running = true
        failure = nil
        send(.importRun)
    }

    /// Whether the first run shows its step and Settings its row: there is data to import (or
    /// data that cannot be read now), or this session's import to report on.
    var offered: Bool {
        if running || imported != nil || failure != nil {
            return true
        }
        return found?.state == .found || found?.state == .unreadable
    }

    /// Whether Import can be pressed.
    var canImport: Bool { offered && !running && imported == nil }

    /// Where it stands, in words: what 0.2 left, why it cannot be read, or what came over.
    var line: String {
        if let imported {
            return "Brought over from Inkwell 0.2: \(Self.summary(imported))."
        }
        if running {
            return "Bringing over what Inkwell 0.2 left\u{2026}"
        }
        if checkFailed && found == nil {
            return "Couldn\u{2019}t look for Inkwell 0.2\u{2019}s data."
        }
        guard let found else { return "" }
        switch found.state {
        case .found:
            return "Inkwell 0.2 left \(found.counts.map(Self.summary) ?? "its history") on this Mac. Import brings them into this library; 0.2\u{2019}s own copy stays as it is."
        case .unreadable:
            return "Inkwell 0.2\u{2019}s data is on this Mac but can\u{2019}t be read now: \(found.message ?? "no reason was given")."
        case .absent, .imported:
            return ""
        }
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .importChecked(let checked) where checked.ref == Self.checkID:
            found = checked
            checkFailed = false
        case .importFinished(let finished) where finished.ref == Self.runID:
            running = false
            imported = finished.counts
            failure = nil
        case .commandFailed(let failed) where failed.command == "import.run" && failed.id == Self.runID:
            running = false
            failure = "Couldn\u{2019}t import: \(failed.message)."
        case .commandFailed(let failed) where failed.command == "import.check" && failed.id == Self.checkID:
            // The first run then goes without the step; Settings says it couldn't look.
            log.write("import.check failed; the import is not offered")
            checkFailed = true
        default:
            break
        }
    }

    /// The ids its commands carry (CoreCommand.json).
    static let checkID = "import.check"
    static let runID = "import.run"

    /// The counts in plain words: "1,204 dictations, 3 snippets, 2 modes and your settings".
    /// linked_keys is 0 in a check (the keychain is not asked then), so keys show only after an
    /// import.
    static func summary(_ counts: ImportCounts) -> String {
        var parts: [String] = []
        func add(_ n: Int64, _ one: String, _ many: String) {
            if n > 0 {
                parts.append("\(n.formatted()) \(n == 1 ? one : many)")
            }
        }
        add(counts.dictations, "dictation", "dictations")
        add(counts.snippets, "snippet", "snippets")
        add(counts.modes, "mode", "modes")
        add(counts.voiceCommands, "voice command", "voice commands")
        add(counts.dictionaryEntries, "dictionary entry", "dictionary entries")
        add(counts.appStyleRules, "app style", "app styles")
        if counts.settings > 0 {
            parts.append("your settings")
        }
        // Linked where they are in the keychain, never copied.
        add(counts.linkedKeys, "saved API key", "saved API keys")
        switch parts.count {
        case 0: return "an empty history"
        case 1: return parts[0]
        default: return parts.dropLast().joined(separator: ", ") + " and " + parts[parts.count - 1]
        }
    }
}

/// Where it stands and the Import button; the failure under it. Shown by the first run's step and
/// Settings > Voice while the model offers an import.
struct Import02Card: View {
    let model: Import02Model

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(model.line)
                    .foregroundStyle(model.checkFailed && model.found == nil ? Theme.alert : Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                if model.canImport {
                    Button("Import") { model.run() }
                        .accessibilityHint("Brings Inkwell 0.2's history and settings into this library")
                }
            }
            if let failure = model.failure {
                Text(failure)
                    .foregroundStyle(Theme.alert)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .contain)
    }
}
