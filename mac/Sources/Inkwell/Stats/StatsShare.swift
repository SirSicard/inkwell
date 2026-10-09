// The share card: an image of the numbers the user ticks, made on this Mac, to copy or save.
// Numbers only (never a word from the library), nothing shared automatically: it leaves the Mac
// only if the user pastes or saves it somewhere. Beside the numbers it can carry a records line,
// a seal for each milestone reached that the user picks, and the heatmap: that last one only once
// the user turns it on (stats.share_heatmap, off unless set), since a grid of days shows which
// days they worked.
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
            // A hidden streak is offered nowhere.
            if d.streakHidden == true { return nil }
            if d.streakDays >= 2 { return ShareLine(value: "\(d.streakDays) active days", label: "dictation streak") }
            return d.longestStreakDays >= 2
                ? ShareLine(value: "\(d.longestStreakDays) active days", label: "longest dictation streak") : nil
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

/// A seal the card can carry: a milestone reached, by its name.
struct ShareSeal: Equatable, Identifiable, Sendable {
    /// The milestone's id.
    let id: String
    /// Its count, short: `10k`, `30`.
    let mark: String
    /// Its name: `A notebook`.
    let name: String

    /// The seals the library has: each milestone reached, in the core's order. A hidden streak's
    /// are not listed by the core.
    static func available(_ c: StatsCounted) -> [ShareSeal] {
        c.milestones.filter(\.reached).compactMap { m in
            m.name.map { ShareSeal(id: m.id, mark: StatsFormat.sealMark(kind: m.kind, threshold: m.threshold), name: StatsFormat.milestoneName($0)) }
        }
    }
}

/// The card's records line: up to three bests, in the core's order.
enum ShareRecords {
    static let count = 3

    /// `Longest dictation: 3 min 12 s · Fastest dictation: 168 wpm · Most words in a day: 2,340`;
    /// nil without a best.
    static func line(_ c: StatsCounted, locale: Locale) -> String? {
        let parts = (c.bests ?? []).prefix(count).map { b in
            let name = StatsFormat.bestName(b.id)
            let value = b.unit == .words ? StatsFormat.count(b.value, locale: locale) : StatsFormat.bestValue(b.value, unit: b.unit, locale: locale)
            // Each record whole on its line: it breaks only between records.
            return "\(name.prefix(1).uppercased() + name.dropFirst()): \(value)".replacingOccurrences(of: " ", with: "\u{00A0}")
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

/// Everything on the card, in its order: the numbers, the records line, the heatmap, the seals.
struct ShareContent: Equatable, Sendable {
    var lines: [ShareLine] = []
    var records: String?
    /// The heatmap's days, shaded 0 to 4, a column per week; nil when not on the card.
    var heatmap: [Int]?
    var seals: [ShareSeal] = []

    var isEmpty: Bool { lines.isEmpty && records == nil && heatmap == nil && seals.isEmpty }

    /// What the card's choices put on it. The heatmap only with `heatmap` (the user's
    /// stats.share_heatmap) and a dictation to show.
    static func make(_ c: StatsCounted, selected: Set<ShareStat>, records: Bool, heatmap: Bool, seals: Set<String>,
                     locale: Locale, calendar: Calendar) -> ShareContent {
        let cells = StatsFormat.heatmap(c.dictation, calendar: calendar)
        return ShareContent(
            lines: ShareStat.lines(c, selected: selected, locale: locale),
            records: records ? ShareRecords.line(c, locale: locale) : nil,
            heatmap: heatmap && c.dictation.dictationsAll > 0 && !cells.isEmpty ? cells.map(\.level) : nil,
            seals: ShareSeal.available(c).filter { seals.contains($0.id) })
    }

    /// What VoiceOver hears for the preview.
    var spoken: String {
        guard !isEmpty else { return "The card is empty" }
        var parts = lines.map { "\($0.value) \($0.label)" }
        if let records { parts.append(records) }
        if heatmap != nil { parts.append("the heatmap of the last 12 weeks") }
        if !seals.isEmpty { parts.append("seals: " + seals.map(\.name).joined(separator: ", ")) }
        return "The card: " + parts.joined(separator: ", ")
    }
}

/// The card itself, in the mode's own colours (fixed swatches, not the window's appearance, so the
/// image is the same wherever it is rendered), with your two colours as a soft glow in its corner.
struct ShareCard: View {
    let content: ShareContent
    let dark: Bool
    let you: Color
    let them: Color

    static let width: CGFloat = 520
    /// A heatmap day's square and the gap between them (the Stats screen's).
    static let cell: CGFloat = 12
    static let gap: CGFloat = 3

    var body: some View {
        let mode = Glow.mode(dark: dark)
        let text = Color(nsColor: mode.text.nsColor)
        let secondary = Color(nsColor: mode.secondary.nsColor)
        VStack(alignment: .leading, spacing: 18) {
            Text("Inkwell")
                .font(.system(size: 22, design: .serif))
                .foregroundStyle(text)
            if !content.lines.isEmpty {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(content.lines, id: \.label) { line in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(line.value)
                                .font(.system(size: 34, design: .serif))
                                .foregroundStyle(text)
                                .monospacedDigit()
                            Text(line.label)
                                .font(.system(size: 14))
                                .foregroundStyle(secondary)
                        }
                    }
                }
            }
            if let records = content.records {
                Text(records)
                    .font(.system(size: 14))
                    .foregroundStyle(text)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let levels = content.heatmap {
                VStack(alignment: .leading, spacing: 6) {
                    heatmap(levels, chip: text.opacity(0.07))
                    Text("Last 12 weeks")
                        .font(.system(size: 12))
                        .foregroundStyle(secondary)
                }
            }
            if !content.seals.isEmpty {
                FlowRow(spacing: 10) {
                    ForEach(content.seals) { seal($0, text: text, secondary: secondary) }
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

    /// The days as the Stats screen shades them, a column per week, in your colour.
    private func heatmap(_ levels: [Int], chip: Color) -> some View {
        let weeks = (levels.count + 6) / 7
        return HStack(alignment: .top, spacing: Self.gap) {
            ForEach(0..<weeks, id: \.self) { week in
                VStack(spacing: Self.gap) {
                    ForEach(0..<7, id: \.self) { weekday in
                        let index = week * 7 + weekday
                        RoundedRectangle(cornerRadius: 3)
                            .fill(index < levels.count
                                ? (levels[index] == 0 ? chip : you.opacity([0, 0.3, 0.5, 0.75, 1][min(max(levels[index], 0), 4)]))
                                : .clear)
                            .frame(width: Self.cell, height: Self.cell)
                    }
                }
            }
        }
    }

    /// A wax seal: the milestone's count in a ring of your colour, its name under it.
    private func seal(_ seal: ShareSeal, text: Color, secondary: Color) -> some View {
        VStack(spacing: 4) {
            ZStack {
                Circle().fill(you.opacity(0.18))
                Circle().strokeBorder(you.opacity(0.75), lineWidth: 1.5)
                Text(seal.mark)
                    .font(.system(size: seal.mark.count > 3 ? 13 : 15, weight: .semibold, design: .serif))
                    .foregroundStyle(text)
                    .monospacedDigit()
            }
            .frame(width: 46, height: 46)
            Text(seal.name)
                .font(.system(size: 10))
                .foregroundStyle(secondary)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .frame(width: 66)
        }
    }
}

/// Making, copying and saving the card.
@MainActor
enum StatsShare {
    /// Pixels per point: sharp on a Retina screen and where it is pasted.
    static let scale: CGFloat = 2

    /// The card as PNG data; nil with nothing to show, or when it could not be drawn.
    static func png(content: ShareContent, dark: Bool, you: Color, them: Color) -> Data? {
        guard !content.isEmpty else { return nil }
        let renderer = ImageRenderer(content: ShareCard(content: content, dark: dark, you: you, them: them))
        renderer.scale = scale
        guard let image = renderer.cgImage else { return nil }
        let rep = NSBitmapImageRep(cgImage: image)
        // Its size in points, so the PNG says 144 dpi and pastes at the card's size, not double.
        rep.size = NSSize(width: CGFloat(image.width) / scale, height: CGFloat(image.height) / scale)
        return rep.representation(using: .png, properties: [:])
    }

    /// Puts the card on `board` as an image (PNG, and TIFF for apps that only take that); whether
    /// it is there.
    @discardableResult
    static func copy(_ png: Data, to board: NSPasteboard = .general) -> Bool {
        guard let image = NSImage(data: png) else { return false }
        board.clearContents()
        let written = board.writeObjects([image])
        return board.setData(png, forType: .png) || written
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
    @State private var records = false
    /// The seals ticked: every one reached, at first.
    @State private var seals: Set<String>?
    @State private var status: String?
    @State private var failed = false
    /// The image as Copy and Save make it, made again only when the ticks or the mode change.
    @State private var png: Data?
    @State private var preview: NSImage?
    /// The sheet's own window, which the save panel attaches to.
    @State private var window: NSWindow?

    private var locale: Locale { stats.calendar.locale ?? .current }
    private var available: [ShareSeal] { ShareSeal.available(counted) }
    private var content: ShareContent {
        ShareContent.make(
            counted, selected: selected, records: records, heatmap: stats.shareHeatmap,
            seals: seals ?? Set(available.map(\.id)), locale: locale, calendar: stats.calendar)
    }

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
                    if let preview {
                        Image(nsImage: preview)
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                            .frame(maxWidth: ShareCard.width * 0.6, maxHeight: Self.previewHeight)
                    } else {
                        Text("Tick a number to put it on the card.")
                            .font(Typography.body)
                            .foregroundStyle(Theme.secondaryText)
                            .frame(width: ShareCard.width * 0.6, height: 160)
                    }
                }
                .frame(width: ShareCard.width * 0.6)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(content.spoken)
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(ShareStat.allCases) { stat in
                        let available = stat.available(in: counted)
                        tick(available ? stat.title : "\(stat.title) (none yet)", enabled: available, isOn: Binding(
                            get: { selected.contains(stat) && available },
                            set: { on in
                                if on { selected.insert(stat) } else { selected.remove(stat) }
                            }))
                    }
                    let hasRecords = ShareRecords.line(counted, locale: locale) != nil
                    tick(hasRecords ? "Records" : "Records (none yet)", enabled: hasRecords,
                         isOn: Binding(get: { records && hasRecords }, set: { records = $0 }))
                    let hasDays = counted.dictation.dictationsAll > 0
                    VStack(alignment: .leading, spacing: 2) {
                        // The user's own choice, kept: off unless they turn it on.
                        tick(hasDays ? "Heatmap of the last 12 weeks" : "Heatmap (none yet)", enabled: hasDays,
                             isOn: Binding(get: { stats.shareHeatmap && hasDays }, set: { stats.setShareHeatmap($0) }))
                        Text("It shows which days you dictated.")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                            .padding(.leading, 20)
                    }
                    if !available.isEmpty {
                        Text("Seals")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                            .padding(.top, 4)
                            .accessibilityAddTraits(.isHeader)
                        FlowRow(spacing: 6) {
                            ForEach(available) { seal in
                                Toggle(seal.name, isOn: Binding(
                                    get: { (seals ?? Set(available.map(\.id))).contains(seal.id) },
                                    set: { on in
                                        var ticked = seals ?? Set(available.map(\.id))
                                        if on { ticked.insert(seal.id) } else { ticked.remove(seal.id) }
                                        seals = ticked
                                        status = nil
                                    }))
                                    .toggleStyle(ChipToggleStyle())
                                    .accessibilityLabel("Seal: \(seal.name)")
                            }
                        }
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
                    .disabled(png == nil)
                Button("Copy") { copy() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(png == nil)
            }
        }
        .padding(24)
        .frame(width: 640)
        .background(WindowReader(window: $window))
        .onAppear(perform: render)
        // The ticks, or new numbers counted while the sheet is open; the mode; your colours.
        .onChange(of: content) { render() }
        .onChange(of: theme.isDark) { render() }
        .onChange(of: theme.settings) { render() }
    }

    /// The preview's tallest: a card with every number, the records, the heatmap and the seals
    /// still leaves the sheet on a laptop screen.
    static let previewHeight: CGFloat = 470

    /// A tick of what the card carries; a new tick clears what Copy or Save said.
    private func tick(_ title: String, enabled: Bool, isOn: Binding<Bool>) -> some View {
        Toggle(isOn: Binding(get: { isOn.wrappedValue }, set: { isOn.wrappedValue = $0; status = nil })) {
            Text(title)
        }
        .toggleStyle(.checkbox)
        .disabled(!enabled)
    }

    private func render() {
        let content = content
        png = StatsShare.png(content: content, dark: theme.isDark, you: theme.you, them: theme.them)
        preview = png.flatMap(NSImage.init(data:))
        if png == nil, !content.isEmpty {
            report("Couldn't make the image.", failed: true)
        }
    }

    private func copy() {
        guard let png, StatsShare.copy(png) else {
            report("Couldn't copy the image.", failed: true)
            return
        }
        report("Copied. Paste it wherever you like.", failed: false)
    }

    private func save() {
        guard let png else { return }
        StatsShare.save(png, from: window) { problem in
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

/// The window a view is in, once it is in one.
private struct WindowReader: NSViewRepresentable {
    @Binding var window: NSWindow?

    final class Reporter: NSView {
        var report: (@MainActor (NSWindow?) -> Void)?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            let window = window
            // After the update that moved it: SwiftUI state is not set during one.
            Task { @MainActor [report] in report?(window) }
        }
    }

    func makeNSView(context: Context) -> Reporter {
        let view = Reporter()
        view.report = { window = $0 }
        return view
    }

    func updateNSView(_ view: Reporter, context: Context) {}
}
