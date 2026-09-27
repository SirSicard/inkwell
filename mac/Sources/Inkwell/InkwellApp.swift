// The app's entry point: an AppKit application, not a SwiftUI App. A SwiftUI Window scene next to
// a menu-bar item fails to present, so AppKit owns the windows (MainWindowController) and the
// menu-bar item (StatusItemController), and SwiftUI draws inside them.
//
// Before anything starts, the single-instance lock: a second copy tells the first to show its
// window and exits without touching the core, the microphone or the dictation key.
import AppKit
import os

@main
enum InkwellMain {
    @MainActor
    static func main() {
        let log = Logger(subsystem: "com.inkwell.app", category: "shell")
        let lockFile = DataLocation.instanceLockFile()
        var lock: InstanceLock?
        do {
            try DataLocation.create(lockFile.deletingLastPathComponent())
            switch InstanceLock.acquire(at: lockFile) {
            case .acquired(let held):
                lock = held
            case .heldElsewhere:
                DistributedNotificationCenter.default().postNotificationName(
                    .inkwellShowWindow, object: nil, userInfo: nil, deliverImmediately: true)
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

        let app = NSApplication.shared
        let delegate = AppDelegate(instanceLock: lock)
        app.delegate = delegate
        // The delegate property is weak: keep ours alive for as long as the app runs.
        withExtendedLifetime(delegate) {
            app.run()
        }
    }
}
