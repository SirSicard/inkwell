// The application object: NSApplication, with a Quit that nothing on screen can refuse.
//
// AppKit ignores terminate: while a window has a sheet attached (the first-run sheet, for one): no
// applicationShouldTerminate, no stop of the core, and the process keeps running. Every way out
// goes through terminate: (the Quit menu items and Cmd-Q, logout and the system's quit request,
// and SIGTERM and SIGINT, which AppDelegate turns into it), so this is the one place that ends
// what would block it first.
//
// It never records anything about what it ends: the delegate's prepareToQuit tells the screens the
// app is quitting (the first-run sheet ended that way stays not completed). The order after it is
// unchanged: applicationShouldTerminate, then the core's stop, which first hands the core the notes
// the screens hold unsaved.
import AppKit

final class InkwellApplication: NSApplication {
    /// Set while an app-modal alert is being stopped for a Quit. A second terminate: in that window
    /// (two signals, or a signal and Cmd-Q) leaves it to the Quit already scheduled instead of
    /// aborting the same modal session twice.
    private var endingModal = false

    override func terminate(_ sender: Any?) {
        if modalWindow != nil {
            guard !endingModal else { return }
            endingModal = true
            // An app-modal alert (Open at Login's error): stop it, and quit once its modal loop has
            // returned. Scheduled in the default mode, which the modal loop does not run.
            abortModal()
            RunLoop.main.perform(inModes: [.default]) {
                MainActor.assumeIsolated { (NSApp as? InkwellApplication)?.quitAfterModal() }
            }
            return
        }
        (delegate as? AppDelegate)?.prepareToQuit()
        for window in windows {
            if let sheet = window.attachedSheet {
                window.endSheet(sheet)
            }
        }
        super.terminate(sender)
    }

    /// The Quit scheduled once an aborted modal loop has returned.
    private func quitAfterModal() {
        endingModal = false
        terminate(nil)
    }
}
