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
    override func terminate(_ sender: Any?) {
        if modalWindow != nil {
            // An app-modal alert (Open at Login's error): stop it, and quit once its modal loop has
            // returned. Scheduled in the default mode, which the modal loop does not run.
            abortModal()
            RunLoop.main.perform(inModes: [.default]) {
                MainActor.assumeIsolated { NSApp.terminate(nil) }
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
}
