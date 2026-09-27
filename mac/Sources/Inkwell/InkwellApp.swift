// The app's entry point: an AppKit application, not a SwiftUI App. A SwiftUI Window scene next to
// a menu-bar item fails to present, so AppKit owns the windows (MainWindowController) and the
// menu-bar item (StatusItemController), and SwiftUI draws inside them.
//
// Before anything starts, the single-instance lock. The copy that holds it listens for show
// requests (ShowWindowChannel) before its core starts; a second copy sends one and exits without
// touching the core, the microphone or the dictation key.
import AppKit
import InkRenderer
import os

@main
enum InkwellMain {
    @MainActor
    static func main() {
        // First, before anything can start a thread (see MetalResidency).
        MetalResidency.configure()
        let log = Logger(subsystem: "com.inkwell.app", category: "shell")
        let lockFile = DataLocation.instanceLockFile()
        let socketPath = DataLocation.instanceSocketFile().path
        let showRequests = ShowRequests()
        var lock: InstanceLock?
        var channel: ShowWindowChannel?
        do {
            try DataLocation.create(lockFile.deletingLastPathComponent())
            switch InstanceLock.acquire(at: lockFile) {
            case .acquired(let held):
                lock = held
                do {
                    channel = try ShowWindowChannel.listen(
                        at: socketPath, onShow: ShowRequests.forwarder(to: showRequests))
                } catch {
                    // The app still runs; a second copy then cannot bring its window forward.
                    log.error("show-window channel: \(String(describing: error), privacy: .public)")
                }
            case .heldElsewhere:
                // The running copy may still be starting: give it a few seconds to listen.
                let outcome = ShowWindowChannel.requestShow(at: socketPath, timeout: 5)
                log.notice("another Inkwell is running; show request: \(String(describing: outcome), privacy: .public)")
                exit(0)
            case .failed(let error):
                // Run anyway, here and below: refusing to start over a lock file would be worse
                // than the rare second copy.
                log.error("the single-instance lock could not be taken (errno \(error, privacy: .public))")
            }
        } catch {
            // Domain and code only: the error's text carries the home directory's path.
            let failure = error as NSError
            log.error("the data directory could not be created (\(failure.domain, privacy: .public) \(failure.code, privacy: .public))")
        }

        // The ink's shader compiles on a background queue from here, alongside the core's start:
        // no window or Drop waits for it on the main thread.
        InkPipelineLoader.shared.warm()

        let app = NSApplication.shared
        let delegate = AppDelegate(instanceLock: lock, showChannel: channel, showRequests: showRequests)
        app.delegate = delegate
        // The delegate property is weak: keep ours alive for as long as the app runs.
        withExtendedLifetime(delegate) {
            app.run()
        }
    }
}
