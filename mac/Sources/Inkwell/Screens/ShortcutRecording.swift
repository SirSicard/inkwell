// The recorder's tie to AppKit: while a shortcut is recorded, the key events of the window that
// shows the recorder go to it (ShortcutRecorderModel) instead of the window, through a local event
// monitor that exists only while recording. Events for any other window pass untouched. The
// window losing key status or closing, the app going to the background, or the section going away
// cancels, so dictation never stays off behind a recorder nobody sees.
import AppKit
import SwiftUI

extension View {
    /// While `recorder` records, key events in this view's window go to it, not to the window.
    func shortcutRecording(_ recorder: ShortcutRecorderModel) -> some View {
        modifier(ShortcutRecording(recorder: recorder))
    }
}

/// The local monitor's filter: the recorder hears a key event only from the window its view is in.
/// Another window's keys (the Drop, an alert, the main window behind Settings), and keys before the
/// view has a window, go on as they were.
@MainActor
enum ShortcutRecordingFilter {
    /// Whether the recorder took the event. `eventWindow` is the event's window, `host` the view's.
    static func feed(
        _ input: ShortcutCapture.Input, from eventWindow: ObjectIdentifier?, host: NSWindow?,
        to recorder: ShortcutRecorderModel
    ) -> Bool {
        guard let host, eventWindow == ObjectIdentifier(host) else { return false }
        return recorder.feed(input)
    }
}

/// The window a view is in, held weakly: the view does not keep it alive.
@MainActor
private final class HostWindow {
    weak var window: NSWindow?
}

private struct ShortcutRecording: ViewModifier {
    let recorder: ShortcutRecorderModel
    @State private var host = HostWindow()
    @State private var monitor: Any?

    func body(content: Content) -> some View {
        content
            .background(WindowReader(host: host))
            .onChange(of: recorder.recording != nil, initial: true) { _, recording in
                if recording { install() } else { remove() }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in
                recorder.cancel()
            }
            .onReceive(NotificationCenter.default.publisher(for: NSWindow.didResignKeyNotification)) { note in
                if let window = note.object as? NSWindow, window === host.window { recorder.cancel() }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { note in
                if let window = note.object as? NSWindow, window === host.window { recorder.cancel() }
            }
            .onDisappear {
                remove()
                recorder.cancel()
            }
    }

    private func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [recorder, host] event in
            let input: ShortcutCapture.Input = event.type == .keyDown
                ? .keyDown(keyCode: Int(event.keyCode), flags: event.modifierFlags.rawValue, isRepeat: event.isARepeat)
                : .flagsChanged(keyCode: Int(event.keyCode), flags: event.modifierFlags.rawValue)
            let eventWindow = event.window.map(ObjectIdentifier.init)
            // A local monitor runs on the main thread, as the app's events are dispatched.
            let taken = MainActor.assumeIsolated {
                ShortcutRecordingFilter.feed(input, from: eventWindow, host: host.window, to: recorder)
            }
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

/// Notes the window its view moves into.
private struct WindowReader: NSViewRepresentable {
    let host: HostWindow

    func makeNSView(context: Context) -> NSView { Probe(host: host) }
    func updateNSView(_ nsView: NSView, context: Context) {}

    final class Probe: NSView {
        let host: HostWindow

        init(host: HostWindow) {
            self.host = host
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            host.window = window
        }
    }
}
