// The main menu, shown while the window is open (the app is a regular app then). Built in code:
// there is no nib and no .xcodeproj. The Edit menu is what makes Cut, Copy, Paste and Undo reach
// text fields, and Cmd-W and Cmd-Q work because these items carry them.
//
//   Inkwell   About, Settings… ⌘,, Check for Updates…, Hide, Quit
//   File      Record Now ⇧⌘R, Stop Recording ⌘.
//   Edit      Undo, Redo, Cut, Copy, Paste, Select All, Find ⌘F
//   View      Today ⌘1, Library ⌘2, Owed ⌘3, Live ⌘4, Show/Hide Sidebar ⌃⌘S, Appearance ▸
//   Window    Minimize, Zoom, Close
//   Help      Keyboard Shortcuts
import AppKit
import InkBridge

@MainActor
enum MainMenu {
    /// `checkForUpdates`: the updater's item, nil when this build does not update itself.
    static func make(checkForUpdates: NSMenuItem?, actions: MenuActions) -> NSMenu {
        let bar = NSMenu()
        bar.addItem(submenu(appMenu(checkForUpdates: checkForUpdates, actions: actions)))
        bar.addItem(submenu(fileMenu(actions)))
        bar.addItem(submenu(editMenu(actions)))
        bar.addItem(submenu(viewMenu(actions)))
        let window = windowMenu()
        bar.addItem(submenu(window))
        NSApp.windowsMenu = window
        let help = helpMenu(actions)
        bar.addItem(submenu(help))
        NSApp.helpMenu = help
        return bar
    }

    private static func submenu(_ menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: menu.title, action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }

    private static func item(
        _ title: String, _ action: Selector, _ key: String, _ modifiers: NSEvent.ModifierFlags = .command,
        target: AnyObject?, tag: Int = 0
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = modifiers
        item.target = target
        item.tag = tag
        return item
    }

    private static func appMenu(checkForUpdates: NSMenuItem?, actions: MenuActions) -> NSMenu {
        let menu = NSMenu(title: "Inkwell")
        menu.addItem(withTitle: "About Inkwell",
                     action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(item("Settings…", #selector(MenuActions.openSettings(_:)), ",", target: actions))
        if let checkForUpdates {
            menu.addItem(checkForUpdates)
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Hide Inkwell", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        let others = menu.addItem(withTitle: "Hide Others",
                                  action: #selector(NSApplication.hideOtherApplications(_:)), keyEquivalent: "h")
        others.keyEquivalentModifierMask = [.command, .option]
        menu.addItem(withTitle: "Show All", action: #selector(NSApplication.unhideAllApplications(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit Inkwell", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        return menu
    }

    private static func fileMenu(_ actions: MenuActions) -> NSMenu {
        let menu = NSMenu(title: "File")
        menu.addItem(item("Record Now", #selector(MenuActions.recordNow(_:)), "r", [.command, .shift], target: actions))
        menu.addItem(item("Stop Recording", #selector(MenuActions.stopRecording(_:)), ".", target: actions))
        return menu
    }

    private static func editMenu(_ actions: MenuActions) -> NSMenu {
        let menu = NSMenu(title: "Edit")
        menu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = menu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(.separator())
        menu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        menu.addItem(.separator())
        menu.addItem(item("Find", #selector(MenuActions.find(_:)), "f", target: actions))
        return menu
    }

    private static func viewMenu(_ actions: MenuActions) -> NSMenu {
        let menu = NSMenu(title: "View")
        for (index, route) in MenuActions.numbered.enumerated() {
            menu.addItem(item(route.title, #selector(MenuActions.showRoute(_:)), "\(index + 1)", target: actions, tag: index))
        }
        menu.addItem(.separator())
        // The split view's own action, sent along the responder chain.
        menu.addItem(item("Show/Hide Sidebar", #selector(NSSplitViewController.toggleSidebar(_:)), "s", [.command, .control], target: nil))
        let appearance = NSMenu(title: "Appearance")
        for (index, mode) in MenuActions.modes.enumerated() {
            appearance.addItem(item(mode == .system ? "Match System" : mode.title,
                                    #selector(MenuActions.setAppearance(_:)), "", [], target: actions, tag: index))
        }
        let appearanceItem = NSMenuItem(title: "Appearance", action: nil, keyEquivalent: "")
        appearanceItem.submenu = appearance
        menu.addItem(appearanceItem)
        return menu
    }

    private static func windowMenu() -> NSMenu {
        let menu = NSMenu(title: "Window")
        menu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        menu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        menu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        return menu
    }

    private static func helpMenu(_ actions: MenuActions) -> NSMenu {
        let menu = NSMenu(title: "Help")
        menu.addItem(item("Keyboard Shortcuts", #selector(MenuActions.showShortcuts(_:)), "", target: actions))
        return menu
    }
}

/// What the main menu's own items do, and when each can be chosen.
@MainActor
final class MenuActions: NSObject, NSMenuItemValidation {
    /// View's numbered routes, ⌘1 to ⌘4.
    static let numbered: [Route] = [.today, .library, .owed, .live]
    /// Appearance's modes, in order.
    static let modes: [GlowTheme.Mode] = [.light, .dark, .system]

    private let router: Router
    private let store: CoreStore
    private let screens: ScreenModels
    private let showWindow: @MainActor () -> Void

    init(router: Router, store: CoreStore, screens: ScreenModels, showWindow: @escaping @MainActor () -> Void) {
        self.router = router
        self.store = store
        self.screens = screens
        self.showWindow = showWindow
    }

    @objc func openSettings(_ sender: Any?) {
        showWindow()
        router.open(.settings)
    }

    @objc func recordNow(_ sender: Any?) {
        screens.meetings.recordNow()
    }

    @objc func stopRecording(_ sender: Any?) {
        screens.meetings.stop()
    }

    @objc func find(_ sender: Any?) {
        showWindow()
        router.focusSearch()
    }

    @objc func showRoute(_ sender: NSMenuItem) {
        guard Self.numbered.indices.contains(sender.tag) else { return }
        showWindow()
        router.open(Self.numbered[sender.tag])
    }

    @objc func setAppearance(_ sender: NSMenuItem) {
        guard Self.modes.indices.contains(sender.tag) else { return }
        screens.theme.setMode(Self.modes[sender.tag])
    }

    @objc func showShortcuts(_ sender: Any?) {
        let alert = NSAlert()
        // An app-modal alert has no window to follow: it takes the mode's appearance itself.
        alert.window.appearance = screens.theme.appearance
        alert.messageText = "Keyboard Shortcuts"
        let key = DictationModel.key(screens.dictation.key)?.name ?? screens.dictation.key
        alert.informativeText = """
            Hold \(key): dictate where your cursor is
            ⇧⌘R: record now
            ⌘.: stop recording
            ⌘1 to ⌘4: Today, Library, Owed, Live
            ⌘,: Settings
            ⌘F: search everything said
            ⌃⌘S: show or hide the sidebar
            Space: play or pause a record
            In Live: ⌘1 to ⌘4 answer what was asked of you, ⌘I asks about the call
            """
        alert.runModal()
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(recordNow(_:)):
            return store.meeting == nil
        case #selector(stopRecording(_:)):
            return store.meeting.map { !$0.stopping } ?? false
        case #selector(showRoute(_:)):
            // Live's own ⌘1 to ⌘4 answer the questions stacked there: the menu stands aside.
            if router.current == .live, !screens.live.stack.questions.isEmpty { return false }
            guard Self.numbered.indices.contains(item.tag) else { return false }
            return Self.numbered[item.tag].isListed(meetingLive: store.meeting != nil)
        case #selector(setAppearance(_:)):
            item.state = Self.modes.indices.contains(item.tag) && Self.modes[item.tag] == screens.theme.settings.mode ? .on : .off
            return true
        default:
            return true
        }
    }
}
