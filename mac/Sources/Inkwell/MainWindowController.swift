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
    /// Reads the calendar's next event for Today's Up next; lives as long as the window.
    private let events: EventKitEvents
    /// Whether the window is on screen, for what redraws on a clock (Up next's minute).
    private let presence = WindowPresence()

    /// `ink` is what the orb and the edge glow show; `updates` is in the environment for the
    /// Settings screen;
    /// `screens` holds the Live, Owed and Settings screens' models (Today reads the permissions
    /// and what is owed from them); `library` feeds Today, the Library and a record.
    init(router: Router, store: CoreStore, ink: ShellInk, updates: Updates, screens: ScreenModels, library: LibraryModel) {
        let events = EventKitEvents()
        let upNext = UpNextModel(access: EventKitCalendar(), events: events)
        events.observe { [weak upNext] in upNext?.refresh() }
        self.events = events
        let root = ShellView(router: router).environment(store).environment(ink).environment(updates)
            .environment(screens).environment(library).environment(upNext).environment(router)
            .environment(presence).environment(screens.theme)
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
        // AppKit restores a saved frame as it was saved, even from a larger display: wider than
        // this screen, an edge sits off it and can't be grabbed. So can the default size on a
        // small screen.
        Self.fitToScreen(window)
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
        // The displays may have changed since launch. A window already up stays where the user
        // put it.
        if let window, !window.isVisible { Self.fitToScreen(window) }
        NSApp.setActivationPolicy(.regular)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate()
        presence.update(window)
    }

    /// Fits the window's frame to its screen's visible frame (WindowFrame), never under the
    /// window's minimum size.
    private static func fitToScreen(_ window: NSWindow) {
        guard let screen = window.screen ?? NSScreen.main else { return }
        let minimum = window.frameRect(forContentRect: NSRect(origin: .zero, size: window.contentMinSize)).size
        let fitted = WindowFrame.fitted(window.frame, in: screen.visibleFrame, minSize: minimum)
        if fitted != window.frame { window.setFrame(fitted, display: false) }
    }

    func windowWillClose(_ notification: Notification) {
        // Back to the menu bar: no Dock icon for an app with no window open.
        NSApp.setActivationPolicy(.accessory)
        presence.update(nil)
    }

    // What changes whether the window is on screen (WindowPresence): covered or uncovered (which
    // includes another Space, the screen locking and the display sleeping), minimised or restored.
    func windowDidChangeOcclusionState(_ notification: Notification) {
        presence.update(window)
    }

    func windowDidMiniaturize(_ notification: Notification) {
        presence.update(window)
    }

    func windowDidDeminiaturize(_ notification: Notification) {
        presence.update(window)
    }
}

/// A window frame fitted to a screen: no wider or taller than the screen's visible frame (the
/// minimum size wins on a screen smaller than it), and wholly on it, moved rather than shrunk
/// where it fits. A frame that is already on screen is left as it is.
enum WindowFrame {
    static func fitted(_ frame: NSRect, in visible: NSRect, minSize: NSSize) -> NSRect {
        let width = max(min(frame.width, visible.width), minSize.width)
        let height = max(min(frame.height, visible.height), minSize.height)
        // Too wide even at the minimum: the left edge on screen. Too tall: the top, which holds
        // the title bar the window is moved by.
        let x = width > visible.width ? visible.minX : min(max(frame.minX, visible.minX), visible.maxX - width)
        let y = height > visible.height ? visible.maxY - height : min(max(frame.minY, visible.minY), visible.maxY - height)
        return NSRect(x: x, y: y, width: width, height: height)
    }
}
