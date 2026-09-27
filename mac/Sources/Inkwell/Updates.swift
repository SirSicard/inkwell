// In-app updates through Sparkle 2: whether this build may update itself, the updater, and the
// hook the Settings screen binds to (`Updates`, in the main window's SwiftUI environment).
//
// Updates run only when the bundle names a feed and the release key: Info.plist's SUFeedURL (an
// https URL) and SUPublicEDKey (the public half of the maintainer's EdDSA key, 32 bytes of
// base64). Until that key has been generated the placeholder is empty, and the app starts no
// updater at all: nothing is fetched and no menu offers a check. `swift run` has no bundle and no
// updater either.
//
// What Sparkle checks before it installs anything (Info.plist asks for the strict forms): the
// archive's EdDSA signature against SUPublicEDKey, before the archive is even unpacked
// (SUVerifyUpdateBeforeExtraction); the feed's own EdDSA signature (SURequireSignedFeed); and that
// the new app is signed by the same Developer ID team as this one. The first automatic check
// waits for the user's answer to Sparkle's own question, and nothing about the Mac is sent with
// a check (SUEnableSystemProfiling is off). How releases are made: docs/RELEASING.md.
import AppKit
import Foundation
import Sparkle

/// Whether this build updates itself, as its Info.plist says.
enum UpdateAvailability: Equatable, Sendable {
    case on(feed: URL)
    case off(Reason)

    /// Why a build does not update itself.
    enum Reason: Equatable, Sendable {
        /// Not an app bundle (`swift run`): there is nothing to replace.
        case notABundle
        /// SUFeedURL is missing, or not an https URL.
        case noFeed
        /// SUPublicEDKey is empty: the release key has not been generated yet.
        case noKey
        /// SUPublicEDKey is not an EdDSA public key: 32 bytes, base64, nothing around it.
        case malformedKey

        /// What the Settings screen says.
        var explanation: String {
            switch self {
            case .notABundle: "Updates work only in the installed app."
            case .noFeed, .noKey, .malformedKey: "This build of Inkwell does not update itself."
            }
        }
    }

    /// Reads `info`, a bundle's Info.plist (nil: no bundle).
    init(infoDictionary info: [String: Any]?) {
        guard let info, info["CFBundleIdentifier"] is String else {
            self = .off(.notABundle)
            return
        }
        guard let feedString = info["SUFeedURL"] as? String,
              let feed = URL(string: feedString),
              feed.scheme?.lowercased() == "https",
              feed.host?.isEmpty == false
        else {
            self = .off(.noFeed)
            return
        }
        let key = info["SUPublicEDKey"] as? String ?? ""
        guard !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            self = .off(.noKey)
            return
        }
        // Strict, as Sparkle decodes it: whitespace pasted around the key makes it unreadable
        // there, so it is refused here too, where a test can see it.
        guard let raw = Data(base64Encoded: key), raw.count == 32 else {
            self = .off(.malformedKey)
            return
        }
        self = .on(feed: feed)
    }
}

/// The updater, and what the Settings screen shows and changes. Main actor, as Sparkle's API is.
///
/// Sparkle keeps its settings in the user's defaults; the properties here read and write them
/// through the updater, and key-value observation tells SwiftUI when Sparkle changes one itself
/// (the first-check question, or a check starting). Nothing polls.
@MainActor
@Observable
final class Updates {
    /// Whether this build updates itself, and why not.
    let availability: UpdateAvailability

    @ObservationIgnored private let controller: SPUStandardUpdaterController?
    @ObservationIgnored private var observations: [NSKeyValueObservation] = []

    private var updater: SPUUpdater? { controller?.updater }

    /// Starts the updater when `infoDictionary` turns updates on; otherwise there is none.
    init(infoDictionary: [String: Any]? = Bundle.main.infoDictionary) {
        availability = UpdateAvailability(infoDictionary: infoDictionary)
        guard case .on = availability else {
            controller = nil
            return
        }
        // Started here: Sparkle schedules its own checks from now on, and asks the user before
        // the first automatic one.
        let controller = SPUStandardUpdaterController(
            startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)
        self.controller = controller
        observations = [
            controller.updater.observe(\.canCheckForUpdates) { [weak self] _, _ in
                Task { @MainActor in self?.sparkleChanged(\.canCheck) }
            },
            controller.updater.observe(\.automaticallyChecksForUpdates) { [weak self] _, _ in
                Task { @MainActor in self?.sparkleChanged(\.checksAutomatically) }
            },
            controller.updater.observe(\.automaticallyDownloadsUpdates) { [weak self] _, _ in
                Task { @MainActor in self?.sparkleChanged(\.downloadsAutomatically) }
            },
        ]
    }

    /// Whether a check can start now: false with updates off, and while a check or an install
    /// is under way.
    var canCheck: Bool {
        access(keyPath: \.canCheck)
        return updater?.canCheckForUpdates ?? false
    }

    /// Check for updates automatically (Sparkle's schedule, daily by default).
    var checksAutomatically: Bool {
        get {
            access(keyPath: \.checksAutomatically)
            return updater?.automaticallyChecksForUpdates ?? false
        }
        set {
            withMutation(keyPath: \.checksAutomatically) {
                updater?.automaticallyChecksForUpdates = newValue
            }
        }
    }

    /// Download and install updates automatically, once they are found.
    var downloadsAutomatically: Bool {
        get {
            access(keyPath: \.downloadsAutomatically)
            return updater?.automaticallyDownloadsUpdates ?? false
        }
        set {
            withMutation(keyPath: \.downloadsAutomatically) {
                updater?.automaticallyDownloadsUpdates = newValue
            }
        }
    }

    /// Checks now, showing Sparkle's own progress and result. Does nothing with updates off.
    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }

    /// A "Check for Updates…" item for a menu; nil with updates off. Sparkle's controller is its
    /// target, and enables it only when a check can start. One item per menu: an item belongs to
    /// one menu at a time.
    func makeMenuItem() -> NSMenuItem? {
        guard let controller else { return nil }
        let item = NSMenuItem(
            title: "Check for Updates…",
            action: #selector(SPUStandardUpdaterController.checkForUpdates(_:)),
            keyEquivalent: "")
        item.target = controller
        return item
    }

    /// Tells observers that Sparkle changed the setting behind `keyPath`.
    private func sparkleChanged<Value>(_ keyPath: KeyPath<Updates, Value>) {
        withMutation(keyPath: keyPath) {}
    }
}
