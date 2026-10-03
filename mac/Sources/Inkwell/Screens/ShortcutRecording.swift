// The recorder's tie to AppKit: while a shortcut is recorded, this window's key events go to the
// recorder (ShortcutRecorderModel) instead of the window, through a local event monitor that
// exists only while recording. Leaving the section or the app cancels, so dictation never stays
// off behind a recorder nobody sees.
import AppKit
import SwiftUI

extension View {
    /// While `recorder` records, key events in this app go to it, not to the window.
    func shortcutRecording(_ recorder: ShortcutRecorderModel) -> some View {
        modifier(ShortcutRecording(recorder: recorder))
    }
}

private struct ShortcutRecording: ViewModifier {
    let recorder: ShortcutRecorderModel
    @State private var monitor: Any?

    func body(content: Content) -> some View {
        content
            .onChange(of: recorder.recording != nil, initial: true) { _, recording in
                if recording { install() } else { remove() }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in
                recorder.cancel()
            }
            .onDisappear {
                remove()
                recorder.cancel()
            }
    }

    private func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [recorder] event in
            let input: ShortcutCapture.Input = event.type == .keyDown
                ? .keyDown(keyCode: Int(event.keyCode), flags: event.modifierFlags.rawValue, isRepeat: event.isARepeat)
                : .flagsChanged(keyCode: Int(event.keyCode), flags: event.modifierFlags.rawValue)
            // A local monitor runs on the main thread, as the app's events are dispatched.
            let taken = MainActor.assumeIsolated { recorder.feed(input) }
            return taken ? nil : event
        }
    }

    private func remove() {
        if let monitor {
            NSEvent.removeMonitor(monitor)
        }
        monitor = nil
    }
}
