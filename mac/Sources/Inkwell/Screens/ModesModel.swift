// Settings > Modes: each mode with how it writes and the apps it is picked for, each app by its
// name and icon.
//
// A mode names apps by identity, as the core matches them: a bundle id (com.example.mail), or part
// of one. That identity is never shown. An installed app is named by NSWorkspace (its display name
// and icon); one that is not installed is named from a short list of well-known apps, and failing
// that as "an app not on this Mac". A fragment that is not an id ("slack") is shown as a word.
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
}

/// One mode's row.
struct ModeRow: Equatable, Identifiable {
    let id: String
    let name: String
    let isDefault: Bool
    /// How it writes: its style, then "Clean up speech" and "Polish" where on.
    let traits: [String]
    /// The apps it is picked for, by name.
    let apps: [AppLabel]
}

@MainActor
@Observable
final class ModesModel {
    private(set) var rows: [ModeRow] = []
    /// The modes could not be read.
    private(set) var failed = false

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let apps: any AppDirectory

    init(send: @escaping SendCommand, apps: any AppDirectory = WorkspaceApps()) {
        self.send = send
        self.apps = apps
    }

    func load() {
        send(.modesList)
    }

    static func style(_ style: ModeStyle) -> String {
        switch style {
        case .formal: "Formal"
        case .casual: "Casual"
        case .relaxed: "Relaxed"
        case .other: "Own style"
        }
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .modesListed(let listed):
            failed = false
            // The default mode last, as "everywhere else": the others are matched first.
            let others = listed.modes.filter { $0.id != listed.defaultId }
            let fallback = listed.modes.filter { $0.id == listed.defaultId }
            rows = (others + fallback).map { mode in
                var traits = [Self.style(mode.style)]
                if mode.removeFillers { traits.append("Clean up speech") }
                if mode.polish { traits.append("Polish") }
                var seen = Set<String>()
                let labels = mode.apps
                    .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
                    .map { AppIdentity.label($0, apps: apps) }
                    .filter { seen.insert($0.id).inserted }
                return ModeRow(
                    id: mode.id, name: mode.name, isDefault: mode.id == listed.defaultId,
                    traits: traits, apps: labels)
            }
        case .commandFailed(let failure) where failure.command == "modes.list":
            failed = true
        default:
            break
        }
    }
}
