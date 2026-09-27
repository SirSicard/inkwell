// Open at login, through SMAppService (the app itself as its login item; no helper bundle).
// Off until the user turns it on: an app that adds itself to login uninvited has overstepped.
import AppKit
import ServiceManagement

@MainActor
enum LoginItem {
    /// What the menu shows.
    enum State: Equatable {
        case on
        case off
        /// Turned on, and waiting for the user in System Settings > General > Login Items.
        case needsApproval
        /// Not running from an app bundle (`swift run`): there is nothing to register.
        case unavailable
    }

    static var state: State {
        guard Bundle.main.bundleIdentifier != nil else { return .unavailable }
        switch SMAppService.mainApp.status {
        case .enabled: return .on
        case .requiresApproval: return .needsApproval
        case .notRegistered, .notFound: return .off
        @unknown default: return .off
        }
    }

    /// Turns it on or off. On failure the state is unchanged and the error says why.
    static func set(_ on: Bool) throws {
        if on {
            try SMAppService.mainApp.register()
        } else {
            try SMAppService.mainApp.unregister()
        }
    }

    /// Opens System Settings at Login Items, where a pending approval is given.
    static func openSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    /// Whether this launch was the login item's (then the app starts in the menu bar only).
    static func launchedAtLogin() -> Bool {
        guard let event = NSAppleEventManager.shared().currentAppleEvent else { return false }
        return event.eventID == kAEOpenApplication
            && event.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
    }
}
