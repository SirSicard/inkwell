// The app's lifecycle: the menu-bar item, the main window, the core, and every way out.
//
// Quitting always stops the core first (ink_shutdown): llama.cpp's Metal backend aborts the
// process at exit if a model is still loaded, so a quit path that skips it crashes on the way
// out. The paths the shell controls are Quit (menu, Cmd-Q, logout: applicationShouldTerminate)
// and SIGTERM/SIGINT (kill, launchd, Ctrl-C in a terminal), which are turned into the same Quit.
import AppKit
import InkBridge
import InkRenderer
import os

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let instanceLock: InstanceLock?
    private let showChannel: ShowWindowChannel?
    private let showRequests: ShowRequests
    private let core = CoreController()
    private let router = Router()
    private let measurement = Measurement.fromEnvironment(ProcessInfo.processInfo.environment)
    /// Created on first use, in applicationDidFinishLaunching: a second copy that exits early
    /// never starts an updater.
    private lazy var updates = Updates()
    private var statusItem: StatusItemController?
    private var mainWindow: MainWindowController?
    /// The ink every surface shows, and the Drop that shows it while something is live.
    private lazy var ink = ShellInk(store: core.store)
    private var drop: DropController?
    private var dropDemo: DropDemo?
    private var signalSources: [DispatchSourceSignal] = []
    private var quitting = false

    /// `instanceLock` is held, and `showChannel` listens, for the life of the process (nil if
    /// either could not be set up). `showRequests` carries the channel's requests to this
    /// delegate, holding any that arrive before it has launched.
    init(instanceLock: InstanceLock?, showChannel: ShowWindowChannel?, showRequests: ShowRequests) {
        self.instanceLock = instanceLock
        self.showChannel = showChannel
        self.showRequests = showRequests
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // With updates off (no release key in this build) there is no updater and no menu item.
        NSApp.mainMenu = MainMenu.make(checkForUpdates: updates.makeMenuItem())
        turnSignalsIntoQuit()

        if let measurement {
            core.observer = { measurement.note($0) }
            measurement.start { [weak self] in self?.mainWindow?.isVisible ?? false }
        }
        core.start()

        // The shell budget's live phase holds the ink live with no audio; the focus check cycles
        // the Drop through its states. Neither is set in ordinary use.
        ink.held = measurement?.heldInk
        InkPipelineLoader.shared.whenReady { [weak self] outcome in
            let log = Logger(subsystem: "com.inkwell.app", category: "ink")
            let took = InkPipelineLoader.shared.compileDuration ?? .zero
            switch outcome {
            case .success:
                log.notice("the ink's shader compiled in \(Int(took / .milliseconds(1)), privacy: .public) ms")
            case .failure(let failure):
                // The app runs on without the ink: every ink zone shows plain paper.
                log.error("the ink cannot draw: \(failure.description, privacy: .public)")
            }
            self?.measurement?.inkReady(outcome, took: took)
        }
        drop = DropController(ink: ink)
        if let interval = DropDemo.interval(from: ProcessInfo.processInfo.environment) {
            dropDemo = DropDemo(ink: ink, interval: interval)
        }

        statusItem = StatusItemController(store: core.store, checkForUpdates: updates.makeMenuItem()) { [weak self] in
            self?.showMainWindow()
        }
        // Opened by the user: show the window. Opened at login: stay in the menu bar, unless a
        // second copy asked for the window while this one was starting (served by attach).
        if !LoginItem.launchedAtLogin() {
            showMainWindow()
        }
        showRequests.attach { [weak self] in
            self?.showMainWindow()
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
        measurement?.windowShown()
        if mainWindow == nil {
            mainWindow = MainWindowController(
                router: router, store: core.store, ink: ink, updates: updates, library: core.library)
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
