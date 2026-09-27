// The player at the foot of a record: play or pause, the two-lane waveform (them above in sepia,
// you below in ink, the played part solid) with the playhead, the time, and the You/Them mix.
// It redraws only while playing.
import InkBridge
import SwiftUI

struct PlayerBar: View {
    @Bindable var player: RecordPlayer
    let document: RecordDocument
    @State private var waveform = Waveform.empty
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Buckets in the waveform: one bar each.
    nonisolated static let bars = 160

    var body: some View {
        HStack(spacing: 16) {
            Button {
                player.toggle()
            } label: {
                Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Paper.card)
                    .frame(width: 38, height: 38)
                    .background(Circle().fill(Paper.you))
            }
            .buttonStyle(.plain)
            // No bare Space shortcut: it would fire while typing in the search field.
            .accessibilityLabel(player.isPlaying ? "Pause" : "Play")
            TimelineView(.animation(minimumInterval: reduceMotion ? 1 : 1.0 / 30, paused: !player.isPlaying)) { _ in
                let position = player.positionMs()
                HStack(spacing: 16) {
                    WaveformView(waveform: waveform, position: position, duration: player.durationMs) { ms in
                        player.seek(toMs: ms)
                    }
                    Text("\(LibraryFormat.stamp(ms: position)) / \(LibraryFormat.stamp(ms: player.durationMs))")
                        .font(PaperType.meta)
                        .foregroundStyle(Paper.quiet)
                        .monospacedDigit()
                        .fixedSize()
                        .accessibilityLabel("\(LibraryFormat.stamp(ms: position)) of \(LibraryFormat.stamp(ms: player.durationMs))")
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                mix("You", value: $player.youVolume, tint: Paper.you, label: "Your volume", enabled: player.sides.contains(.mic))
                mix("Them", value: $player.themVolume, tint: Paper.them, label: "Their volume", enabled: player.sides.contains(.far))
            }
            .fixedSize()
        }
        .padding(.leading, 32)
        .padding(.trailing, 24)
        .frame(height: 72)
        .background(Paper.panel)
        .overlay(alignment: .top) { Rectangle().fill(Paper.hairline).frame(height: 1) }
        .overlay(alignment: .bottomLeading) {
            if case .failed(let why) = player.state {
                Text(why).font(.caption).foregroundStyle(Paper.alert).padding(.leading, 86).padding(.bottom, 2)
            }
        }
        .task(id: document.record.record) {
            let chunks = document.chunks
            let duration = document.durationMs
            // Read off the main actor, a second of audio at a time.
            waveform = await Task.detached(priority: .utility) {
                Waveform.build(chunks: chunks, durationMs: duration, buckets: Self.bars)
            }.value
        }
    }

    private func mix(_ title: String, value: Binding<Float>, tint: Color, label: String, enabled: Bool) -> some View {
        HStack(spacing: 8) {
            Text(title)
                .font(.caption)
                .foregroundStyle(Paper.quiet)
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
        .accessibilityLabel("Waveform: you in ink, them in sepia")
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
                with: .color(Paper.them.opacity(opacity)))
            context.fill(
                Path(roundedRect: CGRect(x: x - barWidth / 2, y: size.height / 2 + 1, width: barWidth, height: youHeight), cornerRadius: 1),
                with: .color(Paper.you.opacity(opacity)))
        }
        let head = CGFloat(fraction) * size.width
        context.fill(Path(CGRect(x: head - 1, y: 0, width: 2, height: size.height)), with: .color(Paper.alert))
    }
}

