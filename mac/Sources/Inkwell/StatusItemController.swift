// The menu-bar item: Inkwell's one always-present surface. Its icon shows the state: the app's
// mark as a template (StatusGlyph), with the live state laid over it (StatusGlyphOverlay, drawn
// from LiveIcon's frames): the orb in your colour while you dictate, breathing in theirs while a
// meeting records, the rim filling with the final pass, the orb in the alert colour when the far
// end has gone quiet. Its menu is rebuilt from the store each time it opens (menuNeedsUpdate), and
// its spoken label follows the ink's state through observation: nothing watches or polls while
// nothing changes.
import AppKit
import InkBridge
import InkRenderer
import Observation

@MainActor
final class StatusItemController: NSObject, NSMenuDelegate, LiveIconSurface {
    private let item: NSStatusItem
    private let store: CoreStore
    private let ink: ShellInk
    private let screens: ScreenModels
    private let openWindow: @MainActor () -> Void
    private let openSettings: @MainActor () -> Void

    private let statusLine = NSMenuItem(title: "", action: nil, keyEquivalent: "")
    private let recordItem = NSMenuItem(title: "Record Now", action: nil, keyEquivalent: "")
    private let dictationItem = NSMenuItem(title: "Dictation", action: nil, keyEquivalent: "")
    private let loginItem = NSMenuItem(title: "Open at Login", action: nil, keyEquivalent: "")
    /// The live state over the mark.
    private let overlay = StatusGlyphOverlay(frame: .zero)
    /// The mark, and the mark without its orb (while the overlay colours the orb).
    private let mark = StatusGlyph.image()
    private let hollowMark = StatusGlyph.image(orb: false)

    /// `checkForUpdates`: the updater's item, nil when this build does not update itself.
    init(
        store: CoreStore, ink: ShellInk, screens: ScreenModels, checkForUpdates: NSMenuItem?,
        openWindow: @escaping @MainActor () -> Void, openSettings: @escaping @MainActor () -> Void
    ) {
        self.store = store
        self.ink = ink
        self.screens = screens
        self.openWindow = openWindow
        self.openSettings = openSettings
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        super.init()

        if let button = item.button {
            button.image = mark
            button.toolTip = "Inkwell"
            overlay.frame = button.bounds
            overlay.autoresizingMask = [.width, .height]
            button.addSubview(overlay)
        }

        let menu = NSMenu()
        menu.delegate = self
        menu.autoenablesItems = false
        statusLine.isEnabled = false
        menu.addItem(statusLine)
        recordItem.target = self
        recordItem.action = #selector(recordOrStop)
        menu.addItem(recordItem)
        dictationItem.target = self
        dictationItem.action = #selector(toggleDictation)
        menu.addItem(dictationItem)
        menu.addItem(.separator())
        let open = NSMenuItem(title: "Open Inkwell", action: #selector(openMainWindow), keyEquivalent: "")
        open.target = self
        menu.addItem(open)
        let settings = NSMenuItem(title: "Settings…", action: #selector(showSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)
        menu.addItem(.separator())
        loginItem.target = self
        loginItem.action = #selector(toggleLoginItem)
        menu.addItem(loginItem)
        if let checkForUpdates {
            menu.addItem(checkForUpdates)
        }
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit Inkwell", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        menu.addItem(quit)
        item.menu = menu
        showState()
    }

    // MARK: The icon

    /// The overlay runs the breath as a layer animation.
    var breathesItself: Bool { overlay.breathesItself }

    func show(_ frame: LiveIconFrame) {
        let image = StatusGlyph.markHasOrb(under: frame.look) ? mark : hollowMark
        if let button = item.button, button.image !== image { button.image = image }
        overlay.show(frame)
    }

    func setAwake(_ awake: Bool) {
        overlay.setAwake(awake)
    }

    /// Keeps the spoken label in line with the ink's state, then waits for the next change (the
    /// change is read on the next turn of the main queue, once applied).
    private func showState() {
        withObservationTracking {
            item.button?.setAccessibilityLabel(Self.spoken(ink.state))
        } onChange: { [weak self] in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.showState() }
            }
        }
    }

    /// What VoiceOver says for the icon.
    static func spoken(_ state: InkState) -> String {
        switch state {
        case .idle: "Inkwell"
        case .dictating: "Inkwell, dictating"
        case .meeting: "Inkwell, recording"
        case .blotting: "Inkwell, finishing a recording"
        case .problem: "Inkwell, recording; the other side is quiet"
        }
    }

    // MARK: The menu

    func menuNeedsUpdate(_ menu: NSMenu) {
        statusLine.title = Self.statusLine(
            status: store.status, meeting: store.meeting, dictation: store.dictation,
            elapsedMs: screens.live.startedAt == nil ? nil : screens.live.elapsedMs())
        if let meeting = store.meeting {
            recordItem.title = "Stop Recording"
            recordItem.isEnabled = !meeting.stopping
        } else {
            recordItem.title = "Record Now"
            recordItem.isEnabled = true
        }
        dictationItem.state = screens.dictation.isOn ? .on : .off
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

    /// "Ready", or "Recording · Design review · 12:04", or what the core is doing.
    static func statusLine(
        status: CoreStore.Status, meeting: CoreStore.LiveMeeting?, dictation: CoreStore.DictationPhase,
        elapsedMs: Int64?
    ) -> String {
        if let meeting {
            let name = meeting.title ?? meeting.appName
            if meeting.stopping {
                return (["Blotting", name] as [String?]).compactMap { $0 }.joined(separator: " · ")
            }
            return (["Recording", name, elapsedMs.map { liveClock(ms: $0) }] as [String?])
                .compactMap { $0 }.joined(separator: " · ")
        }
        if dictation != .idle { return "Dictating" }
        if case .ready = status { return "Ready" }
        return CoreStatusText.line(for: status)
    }

    @objc private func recordOrStop() {
        if store.meeting == nil {
            screens.meetings.recordNow()
        } else {
            screens.meetings.stop()
        }
    }

    @objc private func toggleDictation() {
        screens.dictation.setOn(!screens.dictation.isOn)
    }

    @objc private func openMainWindow() {
        openWindow()
    }

    @objc private func showSettings() {
        openSettings()
    }

    @objc private func toggleLoginItem() {
        LoginItem.toggle()
    }
}

extension LoginItem {
    /// The menu's and Settings' switch: approve a pending one, else turn it the other way. A
    /// failure is shown in an alert.
    static func toggle() {
        switch state {
        case .needsApproval:
            openSettings()
        case .on, .off:
            do {
                try set(state != .on)
                if state == .needsApproval {
                    openSettings()
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
