// A record (the canvas's meeting record): its title and when, then three tabs: notes and
// transcript side by side (your notes first, filled in with what was said; the ledger of who said
// what), the rendered summary, and what is owed. The player sits at the foot: both sides on one
// clock, a two-lane waveform, and a mix. Every timestamp chip and every line plays from its moment.
import AppKit
import InkBridge
import SwiftUI

struct RecordScreen: View {
    @Environment(LibraryModel.self) private var library
    @State private var tab: Tab = .notes

    enum Tab: String, CaseIterable, Identifiable {
        case notes
        case summary
        case owed
        var id: Self { self }
    }

    var body: some View {
        if let document = library.document {
            VStack(spacing: 0) {
                RecordHeader(document: document, tab: $tab)
                Group {
                    switch tab {
                    case .notes: NotesAndTranscript(document: document)
                    case .summary: SummaryTab(document: document)
                    case .owed: OwedTab(document: document)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                if let player = library.player {
                    PlayerBar(player: player, document: document)
                }
            }
        } else if let failure = library.openFailure {
            VStack(spacing: 8) {
                Text("This record can't be opened")
                    .font(.title3.weight(.semibold))
                Text(failure)
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
                Button("Try again") { library.reopen() }
                    .buttonStyle(.link)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ProgressView()
                .controlSize(.small)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .accessibilityLabel("Opening the record")
        }
    }
}

/// Title, when and who, the record's actions, and the tabs.
struct RecordHeader: View {
    let document: RecordDocument
    @Binding var tab: RecordScreen.Tab
    @Environment(LibraryModel.self) private var library

    private var people: [String] {
        document.record.kind == .meeting ? ["You"] + document.people : document.people
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(LibraryFormat.title(of: document.record))
                        .font(PaperType.recordTitle)
                        .foregroundStyle(Theme.text)
                        .lineLimit(3)
                        .accessibilityAddTraits(.isHeader)
                    Text(LibraryFormat.headerLine(
                        document.record, people: people, now: library.now(), calendar: library.calendar))
                        .font(PaperType.meta)
                        .foregroundStyle(Theme.secondaryText)
                }
                Spacer(minLength: 12)
                HStack(spacing: 6) {
                    iconButton("doc.on.doc", label: "Copy summary", disabled: document.summary == nil) {
                        copy(document.summary?.plainText ?? "")
                    }
                    ShareLink(item: shareText) {
                        Image(systemName: "square.and.arrow.up").frame(width: 32, height: 32)
                    }
                    .buttonStyle(.plain)
                    .background(RoundedRectangle(cornerRadius: 8).fill(PaperPalette.card))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(PaperPalette.border, lineWidth: 1))
                    .accessibilityLabel("Share")
                    .help("Share the summary")
                    Menu {
                        Button("Copy transcript") { copy(transcriptText) }
                        Button("Copy summary") { copy(document.summary?.plainText ?? "") }
                            .disabled(document.summary == nil)
                    } label: {
                        Image(systemName: "ellipsis").frame(width: 32, height: 32)
                    }
                    .menuStyle(.button)
                    .menuIndicator(.hidden)
                    .buttonStyle(.plain)
                    .frame(width: 32, height: 32)
                    .background(RoundedRectangle(cornerRadius: 8).fill(PaperPalette.card))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(PaperPalette.border, lineWidth: 1))
                    .accessibilityLabel("More")
                }
            }
            TabRow(tab: $tab, owedCount: document.owed.filter { !$0.done }.count)
        }
        .padding(.top, 22)
        .padding(.horizontal, 32)
    }

    private var shareText: String {
        [LibraryFormat.title(of: document.record), document.summary?.plainText ?? transcriptText]
            .joined(separator: "\n\n")
    }

    private var transcriptText: String {
        document.ledger
            .map { "\(LibraryFormat.stamp(ms: $0.startMs)) \($0.speaker.label): \($0.text)" }
            .joined(separator: "\n")
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func iconButton(_ symbol: String, label: String, disabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).frame(width: 32, height: 32)
        }
        .buttonStyle(.plain)
        .background(RoundedRectangle(cornerRadius: 8).fill(PaperPalette.card))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(PaperPalette.border, lineWidth: 1))
        .disabled(disabled)
        .accessibilityLabel(label)
        .help(label)
    }
}

/// The three tabs, underlined like the canvas's.
struct TabRow: View {
    @Binding var tab: RecordScreen.Tab
    let owedCount: Int

    private func title(_ t: RecordScreen.Tab) -> String {
        switch t {
        case .notes: "Notes and transcript"
        case .summary: "Summary"
        case .owed: owedCount > 0 ? "Owed · \(owedCount)" : "Owed"
        }
    }

    var body: some View {
        HStack(spacing: 22) {
            ForEach(RecordScreen.Tab.allCases) { t in
                let on = tab == t
                Button {
                    tab = t
                } label: {
                    Text(title(t))
                        .font(.callout.weight(on ? .semibold : .regular))
                        .foregroundStyle(on ? Theme.text : Theme.secondaryText)
                        .frame(height: 36)
                        .overlay(alignment: .bottom) {
                            Rectangle().fill(on ? Theme.text : Color.clear).frame(height: 2)
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? [.isSelected] : [])
            }
            Spacer()
        }
        .overlay(alignment: .bottom) { Rectangle().fill(PaperPalette.border).frame(height: 1) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Record")
    }
}

// MARK: Notes and transcript

struct NotesAndTranscript: View {
    let document: RecordDocument
    @Environment(LibraryModel.self) private var library
    @State private var current: Int?

    var body: some View {
        // The canvas's columns: notes 1, transcript 1.12.
        GeometryReader { geometry in
            let notes = max((geometry.size.width - 1) / 2.12, 0)
            HStack(alignment: .top, spacing: 0) {
                MergedNotesView(document: document)
                    .frame(width: notes, alignment: .topLeading)
                Rectangle().fill(PaperPalette.border).frame(width: 1)
                RecordLedgerView(document: document, current: current)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
            }
            .frame(maxHeight: .infinity, alignment: .top)
        }
        .background {
            if let player = library.player {
                PlayheadLine(player: player, document: document, current: $current)
            }
        }
    }
}

/// Follows the playhead while playing and names the line it is in. Redraws only while playing.
struct PlayheadLine: View {
    let player: RecordPlayer
    let document: RecordDocument
    @Binding var current: Int?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(WindowPresence.self) private var presence

    var body: some View {
        // Only while playing and on screen: audio playing behind a covered window draws nothing.
        TimelineView(.animation(minimumInterval: reduceMotion ? 1 : 0.25, paused: !player.isPlaying || !presence.onScreen)) { _ in
            // Marked once the playhead has been put somewhere: by playing, or by a chip or a line.
            let placed = player.state != .idle || player.anchorMs > 0
            let line = placed ? document.line(atPlayhead: player.positionMs())?.id : nil
            Color.clear.task(id: line) { current = line }
        }
        .accessibilityHidden(true)
    }
}

/// Your notes, each followed by what was being said as you wrote it and what was promised.
struct MergedNotesView: View {
    let document: RecordDocument
    @Environment(LibraryModel.self) private var library

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Paper.Eyebrow(text: document.merged.contains { $0.kind == .note } ? "Your notes, filled in" : "Key moments")
                if !document.merged.contains(where: { $0.kind == .note }) {
                    Text(document.record.kind == .meeting ? "You typed no notes in this meeting." : "No notes.")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                }
                ForEach(document.merged) { entry in
                    entryView(entry)
                }
            }
            .padding(.vertical, 20)
            .padding(.leading, 32)
            .padding(.trailing, 26)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityLabel("Notes")
    }

    @ViewBuilder
    private func entryView(_ entry: MergedEntry) -> some View {
        switch entry.kind {
        case .note:
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(entry.text)
                    .font(.system(.body, design: .serif))
                    .foregroundStyle(Theme.text)
                StampChip(ms: entry.atMs) { library.playFrom(entry.atMs) }
            }
            .padding(.top, 4)
        case .said(let speaker):
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("\(Text("\(speaker.label): ").foregroundStyle(speaker.isYou ? PaperPalette.you : PaperPalette.them).bold())\(entry.text)")
                    .font(.system(.callout, design: .serif))
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                StampChip(ms: entry.atMs) { library.playFrom(entry.atMs) }
            }
        case .owed:
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: "checkmark.circle").foregroundStyle(PaperPalette.them).accessibilityLabel("Owed")
                Text(entry.text)
                    .font(.system(.callout, design: .serif))
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                StampChip(ms: entry.atMs) { library.playFrom(entry.atMs) }
            }
        }
    }
}

/// The ledger: every line with its time, who said it and what; the line under the playhead is
/// marked. Clicking a line plays from it.
struct RecordLedgerView: View {
    let document: RecordDocument
    let current: Int?
    @Environment(LibraryModel.self) private var library

    private var status: String {
        document.ledgerStatus(blottedAt: document.summaryWrittenAt.map {
            LibraryFormat.time(LibraryFormat.date(unixMs: $0), calendar: library.calendar)
        })
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 2) {
                HStack {
                    Paper.Eyebrow(text: "What was said")
                    Spacer()
                    Label(status, systemImage: document.isFinal ? "drop.fill" : "drop")
                        .font(PaperType.meta)
                        .foregroundStyle(PaperPalette.quiet)
                        .labelStyle(.titleAndIcon)
                }
                .padding(.leading, 80)
                .padding(.bottom, 10)
                if document.ledger.isEmpty {
                    Text("Nothing was transcribed.")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                        .padding(.leading, 80)
                }
                ForEach(document.ledger) { line in
                    RecordLedgerRow(line: line, current: line.id == current) {
                        library.playFrom(line.startMs)
                    }
                }
            }
            .padding(.vertical, 16)
            .padding(.trailing, 28)
        }
        .accessibilityLabel("Transcript")
    }
}

struct RecordLedgerRow: View {
    let line: LedgerLine
    let current: Bool
    let play: () -> Void

    var body: some View {
        Button(action: play) {
            HStack(alignment: .top, spacing: 8) {
                Text(LibraryFormat.stamp(ms: line.startMs))
                    .font(PaperType.meta)
                    .foregroundStyle(current ? Theme.text : Theme.secondaryText)
                    .frame(width: 64, alignment: .trailing)
                    .padding(.top, 3)
                Circle()
                    .fill(line.speaker.isYou ? PaperPalette.you : PaperPalette.them)
                    .frame(width: 8, height: 8)
                    .padding(.top, 7)
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 2) {
                    Text(line.speaker.label)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(line.speaker.isYou ? PaperPalette.you : PaperPalette.them)
                    Text(line.text)
                        .font(PaperType.reading)
                        .lineSpacing(2)
                        .foregroundStyle(Theme.text)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 8).fill(current ? PaperPalette.chip : Color.clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(LibraryFormat.stamp(ms: line.startMs)), \(line.speaker.label): \(line.text)")
        .accessibilityHint("Plays from here")
        .accessibilityAddTraits(current ? .isSelected : [])
    }
}

// MARK: Summary

struct SummaryTab: View {
    let document: RecordDocument
    @Environment(LibraryModel.self) private var library
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let summary = document.summary {
                    SummaryView(summary: summary)
                        .refusingLinks()
                } else {
                    Text("No summary yet")
                        .font(.headline)
                        .foregroundStyle(Theme.text)
                    Text(screens.summaryOffNote ?? "A summary is written after a meeting ends, when a language model is set up.")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                }
                let decisions = document.summaryItems.filter { $0.kind == .decision }
                if !decisions.isEmpty {
                    Paper.Eyebrow(text: "Decided, and where").padding(.top, 14)
                    ForEach(decisions) { item in
                        CitedItem(text: item.text, citedLine: item.citedLine) { ms in library.playFrom(ms) }
                    }
                }
                let cited = document.owed.filter { $0.citedLine != nil }
                if !cited.isEmpty {
                    Paper.Eyebrow(text: "Where it was said").padding(.top, 14)
                    ForEach(cited) { item in
                        CitedItem(text: item.text, citedLine: item.citedLine) { ms in library.playFrom(ms) }
                    }
                }
            }
            .padding(.vertical, 20)
            .padding(.horizontal, 32)
            .frame(maxWidth: 720, alignment: .leading)
        }
    }
}

/// A rendered summary: the headline, then sections, paragraphs and lists. No markdown token is
/// drawn (SummaryDocument).
struct SummaryView: View {
    let summary: SummaryDocument

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let headline = summary.headline {
                Text(headline)
                    .font(PaperType.lede.weight(.medium))
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(summary.blocks.enumerated()), id: \.offset) { _, block in
                switch block {
                case .heading(let level, let text):
                    Text(text)
                        .font(.system(level <= 2 ? .title3 : .headline, design: .serif, weight: .semibold))
                        .foregroundStyle(Theme.text)
                        .padding(.top, 8)
                        .accessibilityAddTraits(.isHeader)
                case .paragraph(let text):
                    Text(text)
                        .font(PaperType.reading)
                        .lineSpacing(3)
                        .foregroundStyle(Theme.text)
                        .fixedSize(horizontal: false, vertical: true)
                case .item(let marker, let text):
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(marker)
                            .font(PaperType.reading)
                            .foregroundStyle(Theme.secondaryText)
                            .frame(minWidth: 14, alignment: .trailing)
                            .accessibilityHidden(true)
                        Text(text)
                            .font(PaperType.reading)
                            .foregroundStyle(Theme.text)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .textSelection(.enabled)
    }
}

/// A promise from the summary with the line it came from, so a mismatch shows at a glance.
struct CitedItem: View {
    let text: String
    let citedLine: LedgerLine?
    let play: (Int64) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            // Model text (a decision, an action): words only.
            Text(verbatim: text)
                .font(.body.weight(.medium))
                .foregroundStyle(Theme.text)
            if let line = citedLine {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text("\(line.speaker.label): “\(line.text)”")
                        .font(.system(.callout, design: .serif))
                        .foregroundStyle(PaperPalette.quiet)
                        .fixedSize(horizontal: false, vertical: true)
                    StampChip(ms: line.startMs) { play(line.startMs) }
                }
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
    }
}

// MARK: Owed

struct OwedTab: View {
    let document: RecordDocument
    @Environment(LibraryModel.self) private var library

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if document.owed.isEmpty {
                    Text("Nothing was promised in this record.")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                }
                ForEach(document.owed) { item in
                    HStack(alignment: .top, spacing: 12) {
                        Button {
                            library.setDone(item.id, !item.done)
                        } label: {
                            Image(systemName: item.done ? "checkmark.circle.fill" : "circle")
                                .font(.title3)
                                .foregroundStyle(Theme.text)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(item.done ? "Mark not done: \(item.text)" : "Mark done: \(item.text)")
                        VStack(alignment: .leading, spacing: 3) {
                            Text(item.text)
                                .font(.body)
                                .strikethrough(item.done)
                                .foregroundStyle(item.done ? Theme.secondaryText : Theme.text)
                            HStack(spacing: 6) {
                                Text([item.owner, item.due].compactMap { $0 }.joined(separator: " · "))
                                    .font(PaperType.meta)
                                    .foregroundStyle(Theme.secondaryText)
                                if let at = item.atMs {
                                    StampChip(ms: at) { library.playFrom(at) }
                                }
                            }
                        }
                    }
                }
            }
            .padding(.vertical, 20)
            .padding(.horizontal, 32)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
