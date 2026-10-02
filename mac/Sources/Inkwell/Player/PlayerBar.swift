// The player at the foot of a record: play or pause (Space too, while no text field has the
// keyboard), the two-lane waveform (them above, you below, in the theme's colours, the played part
// solid) with the playhead, the time, and the You/Them mix. It redraws only while playing, with
// the window on screen. What it cannot vouch for (an estimated timing, audio it cannot play) is
// said under the waveform, never played as if precise.
import AppKit
import InkBridge
import SwiftUI

struct PlayerBar: View {
    @Bindable var player: RecordPlayer
    let document: RecordDocument
    @State private var loader = WaveformLoader()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(WindowPresence.self) private var presence
    @Environment(GlowTheme.self) private var theme
    @State private var space = SpaceToPlay()

    /// Buckets in the waveform: one bar each.
    nonisolated static let bars = 160

    var body: some View {
        HStack(spacing: 16) {
            Button {
                player.toggle()
            } label: {
                Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Theme.buttonLabel)
                    .frame(width: 38, height: 38)
                    .background(Circle().fill(Theme.buttonFill))
            }
            .buttonStyle(.plain)
            // Space is SpaceToPlay's: a keyboard shortcut would fire while typing in the search field.
            .accessibilityLabel(player.isPlaying ? "Pause" : "Play")
            .help("Play or pause (Space)")
            // Only while playing and on screen: audio playing behind a covered window draws nothing.
            TimelineView(.animation(minimumInterval: reduceMotion ? 1 : 1.0 / 30, paused: !player.isPlaying || !presence.onScreen)) { _ in
                let position = player.positionMs()
                HStack(spacing: 16) {
                    WaveformView(waveform: loader.waveform, position: position, duration: player.durationMs,
                                 youColor: theme.you, themColor: theme.them) { ms in
                        player.seek(toMs: ms)
                    }
                    Text("\(LibraryFormat.stamp(ms: position)) / \(LibraryFormat.stamp(ms: player.durationMs))")
                        .font(PaperType.meta)
                        .foregroundStyle(PaperPalette.quiet)
                        .monospacedDigit()
                        .fixedSize()
                        .accessibilityLabel("\(LibraryFormat.stamp(ms: position)) of \(LibraryFormat.stamp(ms: player.durationMs))")
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                mix("You", value: $player.youVolume, tint: theme.you, label: "Your volume", enabled: player.sides.contains(.mic))
                mix("Them", value: $player.themVolume, tint: theme.them, label: "Their volume", enabled: player.sides.contains(.far))
            }
            .fixedSize()
        }
        .padding(.leading, 32)
        .padding(.trailing, 24)
        .frame(height: 72)
        .background(PaperPalette.panel)
        .overlay(alignment: .top) { Rectangle().fill(PaperPalette.border).frame(height: 1) }
        .overlay(alignment: .bottomLeading) {
            let caveats = document.playbackCaveats.messages(waveformPartial: loader.waveform.partial)
            let failure: String? = if case .failed(let why) = player.state { why } else { nil }
            let lines = [failure].compactMap { $0 } + caveats
            if !lines.isEmpty {
                Text(lines.joined(separator: " "))
                    .font(.caption)
                    .foregroundStyle(PaperPalette.alertText)
                    .lineLimit(1)
                    .padding(.leading, 86)
                    .padding(.bottom, 2)
            }
        }
        .onAppear {
            loader.load(document, buckets: Self.bars)
            space.start { [player] in player.toggle() }
        }
        .onChange(of: document.record.record) { loader.load(document, buckets: Self.bars) }
        .onDisappear {
            loader.cancel()
            space.stop()
        }
    }

    private func mix(_ title: String, value: Binding<Float>, tint: Color, label: String, enabled: Bool) -> some View {
        HStack(spacing: 8) {
            Text(title)
                .font(.caption)
                .foregroundStyle(PaperPalette.quiet)
                .frame(width: 34, alignment: .leading)
                .accessibilityHidden(true)
            Slider(value: value, in: 0...1)
                .tint(tint)
                .frame(width: 78)
                .controlSize(.mini)
                .disabled(!enabled)
                .accessibilityLabel(label)
        }
    }
}

/// Two lanes of bars: them above the middle, you below. Clicking or dragging moves the playhead.
struct WaveformView: View {
    let waveform: Waveform
    let position: Int64
    let duration: Int64
    /// The lanes' colours: yours and theirs.
    let youColor: Color
    let themColor: Color
    let seek: (Int64) -> Void

    private var fraction: Double {
        duration > 0 ? min(max(Double(position) / Double(duration), 0), 1) : 0
    }

    var body: some View {
        GeometryReader { geometry in
            Canvas { context, size in
                draw(in: &context, size: size)
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let width = max(geometry.size.width, 1)
                        seek(Int64(Double(min(max(value.location.x / width, 0), 1)) * Double(duration)))
                    })
        }
        .frame(height: 44)
        .frame(minWidth: 80)
        .accessibilityElement()
        .accessibilityLabel("Waveform: them above, you below")
        .accessibilityValue(LibraryFormat.stamp(ms: position))
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: seek(min(position + 10_000, duration))
            case .decrement: seek(max(position - 10_000, 0))
            @unknown default: break
            }
        }
    }

    private func draw(in context: inout GraphicsContext, size: CGSize) {
        let bars = max(waveform.you.count, waveform.them.count, 1)
        let step = size.width / CGFloat(bars)
        let lane = size.height / 2 - 2
        let played = fraction * Double(bars)
        let barWidth = max(min(2, step - 1), 1)
        for i in 0..<bars {
            let x = CGFloat(i) * step + step / 2
            let opacity = Double(i) < played ? 1 : 0.42
            let them = CGFloat(i < waveform.them.count ? waveform.them[i] : 0)
            let you = CGFloat(i < waveform.you.count ? waveform.you[i] : 0)
            let themHeight = max(them * lane, 1)
            let youHeight = max(you * lane, 1)
            context.fill(
                Path(roundedRect: CGRect(x: x - barWidth / 2, y: size.height / 2 - 1 - themHeight, width: barWidth, height: themHeight), cornerRadius: 1),
                with: .color(themColor.opacity(opacity)))
            context.fill(
                Path(roundedRect: CGRect(x: x - barWidth / 2, y: size.height / 2 + 1, width: barWidth, height: youHeight), cornerRadius: 1),
                with: .color(youColor.opacity(opacity)))
        }
        let head = CGFloat(fraction) * size.width
        context.fill(Path(CGRect(x: head - 1, y: 0, width: 2, height: size.height)), with: .color(PaperPalette.alertText))
    }
}


/// Space plays and pauses the open record while no text field has the keyboard (the search field,
/// a note being typed): a handler for the main window's own key presses, there only while the
/// player is. Typing, and every other key, goes where it always went.
@MainActor
final class SpaceToPlay {
    private var monitor: Any?

    func start(_ toggle: @escaping @MainActor () -> Void) {
        guard monitor == nil else { return }
        // Local monitors run on the main thread.
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard Self.isPlayKey(event) else { return event }
            toggle()
            return nil
        }
    }

    func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    /// A bare Space in a window that is not typing: not in a text field or a text view (the field
    /// editor is one), not on a focused control, and not in a panel (the Drop, a sheet's panel).
    /// With Full Keyboard Access on, Space presses the focused control, as the user expects: it is
    /// left alone then.
    static func isPlayKey(_ event: NSEvent) -> Bool {
        guard !NSApp.isFullKeyboardAccessEnabled else { return false }
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask).subtracting([.capsLock, .numericPad, .function])
        guard event.keyCode == 49, modifiers.isEmpty, !event.isARepeat,
              let window = event.window, window.isKeyWindow, window.attachedSheet == nil, !(window is NSPanel)
        else { return false }
        let responder = window.firstResponder
        return !(responder is NSText) && !(responder is NSControl)
    }
}
