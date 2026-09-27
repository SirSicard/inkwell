// The main window, created and owned by AppKit. SwiftUI draws its content through an
// NSHostingController; the window itself is never a SwiftUI scene, because a SwiftUI Window next
// to a menu-bar item fails to present.
//
// Inkwell lives in the menu bar (LSUIElement). While this window is open the app becomes a
// regular app, with a Dock icon, a main menu and a place in Cmd-Tab; when it closes the app goes
// back to the menu bar. Dictation never depends on the window.
import AppKit
import InkBridge
import SwiftUI

@MainActor
final class MainWindowController: NSWindowController, NSWindowDelegate {
    /// `updates` is in the environment for the Settings screen.
    init(router: Router, store: CoreStore, updates: Updates) {
        let root = ShellView(router: router).environment(store).environment(updates)
        let hosting = NSHostingController(rootView: root)
        // The SwiftUI title and toolbar become the window's; the sidebar toggle lives there.
        hosting.sceneBridgingOptions = [.title, .toolbars]
        // SwiftUI sets the minimum size; the user sets the rest.
        hosting.sizingOptions = [.minSize]

        let window = NSWindow(contentViewController: hosting)
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbarStyle = .unified
        window.title = "Inkwell"
        window.backgroundColor = Theme.windowBackground
        window.isReleasedWhenClosed = false
        window.setContentSize(NSSize(width: 1040, height: 700))
        window.contentMinSize = NSSize(width: 720, height: 460)
        window.center()
        // After center(): a saved frame, when there is one, wins.
        window.setFrameAutosaveName("Inkwell.main")
        super.init(window: window)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    /// Whether the window is on screen and not fully covered.
    var isVisible: Bool {
        guard let window else { return false }
        return window.isVisible && window.occlusionState.contains(.visible)
    }

    /// Brings the window up, in front, with the app active.
    func present() {
        NSApp.setActivationPolicy(.regular)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate()
    }

    func windowWillClose(_ notification: Notification) {
        // Back to the menu bar: no Dock icon for an app with no window open.
        NSApp.setActivationPolicy(.accessory)
    }
}
