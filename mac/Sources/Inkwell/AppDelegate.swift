// The app's lifecycle: the menu-bar item, the main window, the core, and every way out.
//
// Quitting always stops the core first (ink_shutdown): llama.cpp's Metal backend aborts the
// process at exit if a model is still loaded, so a quit path that skips it crashes on the way
// out. The paths the shell controls are Quit (menu, Cmd-Q, logout: applicationShouldTerminate)
// and SIGTERM/SIGINT (kill, launchd, Ctrl-C in a terminal), which are turned into the same Quit.
import AppKit
import InkBridge
import os

extension Notification.Name {
    /// Posted by a second copy of Inkwell as it exits: the running one shows its window.
    static let inkwellShowWindow = Notification.Name("com.inkwell.app.show-window")
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let instanceLock: InstanceLock?
    private let core = CoreController()
    private let router = Router()
    private let measurement = Measurement.fromEnvironment(ProcessInfo.processInfo.environment)
    private var statusItem: StatusItemController?
    private var mainWindow: MainWindowController?
    private var signalSources: [DispatchSourceSignal] = []
    private var quitting = false

    /// `instanceLock` is held for the life of the process (nil if it could not be taken).
    init(instanceLock: InstanceLock?) {
        self.instanceLock = instanceLock
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = MainMenu.make()
        turnSignalsIntoQuit()

        if let measurement {
            core.observer = { measurement.note($0) }
            measurement.start { [weak self] in self?.mainWindow?.isVisible ?? false }
        }
        core.start()

        statusItem = StatusItemController(store: core.store) { [weak self] in
            self?.showMainWindow()
        }
        DistributedNotificationCenter.default().addObserver(
            forName: .inkwellShowWindow, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.showMainWindow() }
        }

        // Opened by the user: show the window. Opened at login: stay in the menu bar.
        if !LoginItem.launchedAtLogin() {
            showMainWindow()
        }
    }

    /// Clicked in the Dock or opened again from the Finder while running.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return false
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard core.isRunning || quitting else { return .terminateNow }
        // A second Quit while the core is stopping waits for the same stop.
        guard !quitting else { return .terminateLater }
        quitting = true
        core.stop {
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    private func showMainWindow() {
        if mainWindow == nil {
            mainWindow = MainWindowController(router: router, store: core.store)
        }
        mainWindow?.present()
    }

    /// SIGTERM and SIGINT become an ordinary Quit, so they stop the core like any other.
    ///
    /// The Quit runs as a run-loop block, outside the signal source's main-queue block: Quit waits
    /// in a nested run loop, and from inside a main-queue block that loop could run no other
    /// main-queue block (the event relay's among them) until Quit returned.
    private func turnSignalsIntoQuit() {
        for number in [SIGTERM, SIGINT] {
            signal(number, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
            source.setEventHandler {
                RunLoop.main.perform {
                    MainActor.assumeIsolated { NSApp.terminate(nil) }
                }
            }
            source.resume()
            signalSources.append(source)
        }
    }
}
