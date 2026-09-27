// The menu-bar item: Inkwell's one always-present surface. Its menu is rebuilt from the store each
// time it opens (menuNeedsUpdate), so nothing watches or polls while it is closed.
import AppKit
import InkBridge

@MainActor
final class StatusItemController: NSObject, NSMenuDelegate {
    private let item: NSStatusItem
    private let store: CoreStore
    private let openWindow: @MainActor () -> Void

    private let statusLine = NSMenuItem(title: "", action: nil, keyEquivalent: "")
    private let loginItem = NSMenuItem(title: "Open at Login", action: nil, keyEquivalent: "")

    /// `checkForUpdates`: the updater's item, nil when this build does not update itself.
    init(store: CoreStore, checkForUpdates: NSMenuItem?, openWindow: @escaping @MainActor () -> Void) {
        self.store = store
        self.openWindow = openWindow
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        super.init()

        if let button = item.button {
            let image = NSImage(systemSymbolName: "drop.fill", accessibilityDescription: "Inkwell")
            image?.isTemplate = true
            button.image = image
            button.toolTip = "Inkwell"
        }

        let menu = NSMenu()
        menu.delegate = self
        statusLine.isEnabled = false
        menu.addItem(statusLine)
        menu.addItem(.separator())
        let open = NSMenuItem(title: "Open Inkwell", action: #selector(openMainWindow), keyEquivalent: "")
        open.target = self
        menu.addItem(open)
        menu.addItem(.separator())
        loginItem.target = self
        loginItem.action = #selector(toggleLoginItem)
        menu.addItem(loginItem)
        if let checkForUpdates {
            menu.addItem(checkForUpdates)
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit Inkwell", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        item.menu = menu
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        statusLine.title = CoreStatusText.line(for: store.status)
        switch LoginItem.state {
        case .on:
            loginItem.state = .on
            loginItem.title = "Open at Login"
            loginItem.isEnabled = true
        case .off:
            loginItem.state = .off
            loginItem.title = "Open at Login"
            loginItem.isEnabled = true
        case .needsApproval:
            loginItem.state = .mixed
            loginItem.title = "Open at Login (approve in System Settings)"
            loginItem.isEnabled = true
        case .unavailable:
            loginItem.state = .off
            loginItem.title = "Open at Login"
            loginItem.isEnabled = false
        }
    }

    @objc private func openMainWindow() {
        openWindow()
    }

    @objc private func toggleLoginItem() {
        switch LoginItem.state {
        case .needsApproval:
            LoginItem.openSettings()
        case .on, .off:
            do {
                try LoginItem.set(LoginItem.state != .on)
                if LoginItem.state == .needsApproval {
                    LoginItem.openSettings()
                }
            } catch {
                let alert = NSAlert(error: error)
                alert.messageText = "Inkwell could not change Open at Login."
                alert.runModal()
            }
        case .unavailable:
            break
        }
    }
}
