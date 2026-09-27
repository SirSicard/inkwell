// The main menu, shown while the window is open (the app is a regular app then). Built in code:
// there is no nib and no .xcodeproj. The Edit menu is what makes Cut, Copy, Paste and Undo reach
// text fields, and Cmd-W and Cmd-Q work because these items carry them.
import AppKit

@MainActor
enum MainMenu {
    /// `checkForUpdates`: the updater's item, nil when this build does not update itself.
    static func make(checkForUpdates: NSMenuItem?) -> NSMenu {
        let bar = NSMenu()
        bar.addItem(submenu(appMenu(checkForUpdates: checkForUpdates)))
        bar.addItem(submenu(editMenu()))
        let window = windowMenu()
        bar.addItem(submenu(window))
        NSApp.windowsMenu = window
        return bar
    }

    private static func submenu(_ menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: menu.title, action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }

    private static func appMenu(checkForUpdates: NSMenuItem?) -> NSMenu {
        let menu = NSMenu(title: "Inkwell")
        menu.addItem(withTitle: "About Inkwell",
                     action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
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

    private static func editMenu() -> NSMenu {
        let menu = NSMenu(title: "Edit")
        menu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = menu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(.separator())
        menu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        return menu
    }

    private static func windowMenu() -> NSMenu {
        let menu = NSMenu(title: "Window")
        menu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        menu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        menu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        return menu
    }
}
