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
    /// The main menu's own items' target.
    private var menuActions: MenuActions?
    private var mainWindow: MainWindowController?
    /// The ink every surface shows, and the Drop that shows it while something is live.
    private lazy var ink = ShellInk(
        store: core.store, permissions: core.screens.permissions, meetings: core.screens.meetings)
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
        menuActions = MenuActions(router: router, store: core.store, screens: core.screens) { [weak self] in
            self?.showMainWindow()
        }
        if let menuActions {
            NSApp.mainMenu = MainMenu.make(checkForUpdates: updates.makeMenuItem(), actions: menuActions)
        }
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
        // The theme follows the system's appearance and accessibility settings from now on.
        core.screens.theme.start()
        let drop = DropController(ink: ink, notes: core.screens.dictation, theme: core.screens.theme)
        let screens = core.screens
        drop.onAction = { [weak self] action in
            screens.performDropAction(action) { route in
                self?.showMainWindow()
                self?.router.open(route)
            }
        }
        self.drop = drop
        if let interval = DropDemo.interval(from: ProcessInfo.processInfo.environment) {
            dropDemo = DropDemo(ink: ink, interval: interval)
        }

        statusItem = StatusItemController(
            store: core.store, ink: ink, screens: core.screens, checkForUpdates: updates.makeMenuItem(),
            openWindow: { [weak self] in self?.showMainWindow() },
            openSettings: { [weak self] in
                self?.showMainWindow()
                self?.router.open(.settings)
            })
        // Opened by the user: show the window. Opened at login: stay in the menu bar, unless a
        // second copy asked for the window while this one was starting (served by attach).
        if !LoginItem.launchedAtLogin() {
            showMainWindow()
        }
        showRequests.attach { [weak self] in
            self?.showMainWindow()
        }
        // Inkwell 0.2's Open at Login, carried over once; last, as a failure is shown in an alert.
        LoginItemMigration.runAtLaunch()
    }

    /// Clicked in the Dock or opened again from the Finder while running.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return false
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    /// Quit is on its way (InkwellApplication.terminate): the screens let go of what would block it,
    /// recording nothing (the first-run sheet ended this way shows again at the next launch).
    func prepareToQuit() {
        core.screens.onboarding.appQuitting()
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
                router: router, store: core.store, ink: ink, updates: updates, screens: core.screens,
                library: core.library)
        }
        mainWindow?.present()
    }

    /// SIGTERM and SIGINT become an ordinary Quit, so they stop the core like any other.
    ///
    /// The signal is taken on a background queue and the Quit handed to the main run loop as a
    /// run-loop block in the common modes. Not a main-queue block: Quit waits in a nested run loop,
    /// and from inside a main-queue block that loop could run no other main-queue block (the event
    /// relay's among them) until Quit returned; and while an app-modal alert runs, the main queue is
    /// not served at all (measured on macOS 27), while a common-modes run-loop block is
    /// (InkwellApplication.terminate then ends the alert).
    /// **Signal queue.** Hands Quit to the main run loop, in the common modes, and wakes it.
    nonisolated private static func quitOnMainRunLoop() {
        let main = CFRunLoopGetMain()
        CFRunLoopPerformBlock(main, CFRunLoopMode.commonModes.rawValue) {
            MainActor.assumeIsolated { NSApp.terminate(nil) }
        }
        CFRunLoopWakeUp(main)
    }

    private func turnSignalsIntoQuit() {
        for number in [SIGTERM, SIGINT] {
            signal(number, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: number, queue: .global(qos: .userInitiated))
            // A nonisolated function, not a closure written here: a closure in this main-actor
            // method would be main-actor isolated, and running it on the signal queue traps.
            source.setEventHandler(handler: Self.quitOnMainRunLoop)
            source.resume()
            signalSources.append(source)
        }
    }
}
