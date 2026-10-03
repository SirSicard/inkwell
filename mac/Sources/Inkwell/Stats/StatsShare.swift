// The share card: an image of the numbers the user ticks, made on this Mac, to copy or save.
// Numbers only (never a word from the library), nothing shared automatically: it leaves the Mac
// only if the user pastes or saves it somewhere.
import AppKit
import InkBridge
import SwiftUI
import UniformTypeIdentifiers

/// A number the card can carry.
enum ShareStat: String, CaseIterable, Identifiable, Sendable {
    case wordsAll
    case wordsWeek
    case speed
    case saved
    case streak
    case meetingHours
    case talkTime
    case promisesKept

    var id: String { rawValue }

    /// The tick's name.
    var title: String {
        switch self {
        case .wordsAll: "Words dictated"
        case .wordsWeek: "Words this week"
        case .speed: "Speaking speed this week"
        case .saved: "Time saved"
        case .streak: "Streak"
        case .meetingHours: "Hours in meetings this month"
        case .talkTime: "Talk time this month"
        case .promisesKept: "Promises kept this month"
        }
    }

    /// What the card carries at first.
    static let defaults: Set<ShareStat> = [.wordsAll, .saved, .streak]

    /// Its line on the card, or nil while the library has no such number yet.
    func line(_ c: StatsCounted, locale: Locale) -> ShareLine? {
        let d = c.dictation
        switch self {
        case .wordsAll:
            return d.dictationsAll > 0 ? ShareLine(value: StatsFormat.count(d.wordsAll, locale: locale), label: "words dictated") : nil
        case .wordsWeek:
            return d.dictationsAll > 0 ? ShareLine(value: StatsFormat.count(d.wordsWeek, locale: locale), label: "words this week") : nil
        case .speed:
            return d.wpmWeek.map { ShareLine(value: "\($0) wpm", label: "speaking speed this week") }
        case .saved:
            return d.savedMsAll > 0
                ? ShareLine(value: LibraryFormat.duration(ms: d.savedMsAll), label: "saved vs typing at \(c.typingWpm) wpm") : nil
        case .streak:
            if d.streakDays >= 2 { return ShareLine(value: "\(d.streakDays) days", label: "dictation streak") }
            return d.longestStreakDays >= 2 ? ShareLine(value: "\(d.longestStreakDays) days", label: "longest dictation streak") : nil
        case .meetingHours:
            let m = c.meetingsMonth
            return m.meetings > 0 ? ShareLine(value: LibraryFormat.duration(ms: m.recordedMs), label: "in meetings this month") : nil
        case .talkTime:
            return StatsFormat.talkShare(you: c.meetingsMonth.youMs, them: c.meetingsMonth.themMs).map {
                ShareLine(value: "\($0.you) % / \($0.them) %", label: "talk time this month, you / them")
            }
        case .promisesKept:
            let p = c.promisesMonth
            return p.made > 0 ? ShareLine(value: "\(p.kept) of \(p.made)", label: "promises kept this month") : nil
        }
    }

    /// Whether the library has this number yet.
    func available(in c: StatsCounted) -> Bool {
        line(c, locale: .current) != nil
    }

    /// The card's lines: the ticked ones the library has, in the card's own order.
    static func lines(_ c: StatsCounted, selected: Set<ShareStat>, locale: Locale) -> [ShareLine] {
        allCases.filter(selected.contains).compactMap { $0.line(c, locale: locale) }
    }
}

/// One line of the card: a number and what it counts.
struct ShareLine: Equatable, Sendable {
    let value: String
    let label: String
}

/// The card itself, in the mode's own colours (fixed swatches, not the window's appearance, so the
/// image is the same wherever it is rendered), with your two colours as a soft glow in its corner.
struct ShareCard: View {
    let lines: [ShareLine]
    let dark: Bool
    let you: Color
    let them: Color

    static let width: CGFloat = 520

    var body: some View {
        let mode = Glow.mode(dark: dark)
        VStack(alignment: .leading, spacing: 18) {
            Text("Inkwell")
                .font(.system(size: 22, design: .serif))
                .foregroundStyle(Color(nsColor: mode.text.nsColor))
            VStack(alignment: .leading, spacing: 14) {
                ForEach(lines, id: \.label) { line in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(line.value)
                            .font(.system(size: 34, design: .serif))
                            .foregroundStyle(Color(nsColor: mode.text.nsColor))
                            .monospacedDigit()
                        Text(line.label)
                            .font(.system(size: 14))
                            .foregroundStyle(Color(nsColor: mode.secondary.nsColor))
                    }
                }
            }
            Text("Counted on my Mac")
                .font(.system(size: 12))
                .foregroundStyle(Color(nsColor: mode.secondary.nsColor))
        }
        .padding(36)
        .frame(width: Self.width, alignment: .leading)
        .background {
            ZStack(alignment: .topTrailing) {
                Color(nsColor: mode.background.nsColor)
                Circle()
                    .fill(RadialGradient(colors: [you.opacity(0.55), them.opacity(0.3), .clear], center: .center,
                                         startRadius: 0, endRadius: 170))
                    .frame(width: 340, height: 340)
                    .offset(x: 110, y: -120)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 22))
    }
}

/// Making, copying and saving the card.
@MainActor
enum StatsShare {
    /// Pixels per point: sharp on a Retina screen and where it is pasted.
    static let scale: CGFloat = 2

    /// The card as PNG data; nil with nothing to show, or when it could not be drawn.
    static func png(lines: [ShareLine], dark: Bool, you: Color, them: Color) -> Data? {
        guard !lines.isEmpty else { return nil }
        let renderer = ImageRenderer(content: ShareCard(lines: lines, dark: dark, you: you, them: them))
        renderer.scale = scale
        guard let image = renderer.cgImage else { return nil }
        return NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
    }

    /// Puts the card on `board` as an image (PNG, and TIFF for apps that only take that).
    static func copy(_ png: Data, to board: NSPasteboard = .general) {
        board.clearContents()
        if let image = NSImage(data: png) {
            board.writeObjects([image])
        }
        board.setData(png, forType: .png)
    }

    /// Asks where to save the card, then writes it there. `done` gets nil when saved, or what went
    /// wrong in words; it is not called when the user cancels.
    static func save(_ png: Data, from window: NSWindow?, done: @escaping @MainActor (String?) -> Void) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.png]
        panel.nameFieldStringValue = "Inkwell stats.png"
        panel.canCreateDirectories = true
        let write: @MainActor (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try png.write(to: url, options: .atomic)
                done(nil)
            } catch {
                done(error.localizedDescription)
            }
        }
        if let window {
            panel.beginSheetModal(for: window) { response in MainActor.assumeIsolated { write(response) } }
        } else {
            write(panel.runModal())
        }
    }
}

/// The sheet: the card as it will look, the ticks for what it carries, and Copy or Save.
struct ShareCardSheet: View {
    let counted: StatsCounted
    let stats: StatsModel
    @Environment(GlowTheme.self) private var theme
    @Environment(\.dismiss) private var dismiss
    @State private var selected = ShareStat.defaults
    @State private var status: String?
    @State private var failed = false

    private var locale: Locale { stats.calendar.locale ?? .current }
    private var lines: [ShareLine] { ShareStat.lines(counted, selected: selected, locale: locale) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Share card")
                .font(Typography.heading)
                .foregroundStyle(Theme.text)
                .accessibilityAddTraits(.isHeader)
            Text("Numbers only, made on this Mac. Nothing leaves it unless you paste or save the image somewhere.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            HStack(alignment: .top, spacing: 24) {
                Group {
                    // The image itself, as Copy and Save make it.
                    if let preview = image().flatMap(NSImage.init(data:)) {
                        Image(nsImage: preview)
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                            .frame(width: ShareCard.width * 0.6)
                    } else {
                        Text("Tick a number to put it on the card.")
                            .font(Typography.body)
                            .foregroundStyle(Theme.secondaryText)
                            .frame(width: ShareCard.width * 0.6, height: 160)
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(lines.isEmpty
                    ? "The card is empty"
                    : "The card: " + lines.map { "\($0.value) \($0.label)" }.joined(separator: ", "))
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(ShareStat.allCases) { stat in
                        let available = stat.available(in: counted)
                        Toggle(isOn: Binding(
                            get: { selected.contains(stat) && available },
                            set: { on in
                                if on { selected.insert(stat) } else { selected.remove(stat) }
                                status = nil
                            })
                        ) {
                            Text(available ? stat.title : "\(stat.title) (none yet)")
                        }
                        .toggleStyle(.checkbox)
                        .disabled(!available)
                    }
                }
                .font(Typography.body)
            }
            HStack(spacing: 10) {
                if let status {
                    Text(status)
                        .font(Typography.caption)
                        .foregroundStyle(failed ? Theme.alert : Theme.secondaryText)
                }
                Spacer(minLength: 0)
                Button("Done") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save\u{2026}") { save() }
                    .disabled(lines.isEmpty)
                Button("Copy") { copy() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(lines.isEmpty)
            }
        }
        .padding(24)
        .frame(width: 640)
    }

    private func image() -> Data? {
        StatsShare.png(lines: lines, dark: theme.isDark, you: theme.you, them: theme.them)
    }

    private func copy() {
        guard let png = image() else {
            report("Couldn't make the image.", failed: true)
            return
        }
        StatsShare.copy(png)
        report("Copied. Paste it wherever you like.", failed: false)
    }

    private func save() {
        guard let png = image() else {
            report("Couldn't make the image.", failed: true)
            return
        }
        StatsShare.save(png, from: NSApp.keyWindow) { problem in
            if let problem {
                report("Couldn't save it: \(problem)", failed: true)
            } else {
                report("Saved.", failed: false)
            }
        }
    }

    private func report(_ text: String, failed: Bool) {
        status = text
        self.failed = failed
        AccessibilityNotification.Announcement(text).post()
    }
}
