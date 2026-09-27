// Live: the meeting being recorded. Your notes on the left (a TextKit 2 editor), what is being
// said on the right (the ledger: dry finals, wet partials), and Ask along the bottom with the far
// end's questions stacked above it.
//
// Nothing here ticks while idle: the clock in the header redraws once a second only while this
// screen shows a live meeting (the ink is drawing then anyway).
import AppKit
import InkBridge
import SwiftUI

struct LiveScreen: View {
    @Environment(CoreStore.self) private var store
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        if let meeting = store.meeting {
            LiveMeetingView(meeting: meeting, live: screens.live, meetings: screens.meetings)
        } else {
            VStack(alignment: .leading, spacing: 12) {
                Paper.Header(title: "Live", subtitle: nil)
                Text("No meeting is being recorded.")
                    .font(Typography.body)
                    .foregroundStyle(Theme.secondaryText)
                Button("Record now") { screens.meetings.recordNow() }
                    .buttonStyle(PaperButtonStyle(prominent: true))
                if let failure = screens.meetings.failure {
                    Text("Couldn't start recording: \(failure)")
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                }
                Spacer()
            }
            .padding(32)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }
}

struct LiveMeetingView: View {
    let meeting: CoreStore.LiveMeeting
    @Bindable var live: LiveModel
    let meetings: MeetingModel

    var body: some View {
        let lines = LiveLine.ledger(meeting)
        VStack(alignment: .leading, spacing: 18) {
            header
            HStack(alignment: .top, spacing: 0) {
                // Minimum widths: a text squeezed to no width would be as tall as its letters, and
                // the window follows this screen's minimum size.
                notes
                    .frame(minWidth: 200, maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .padding(.trailing, 26)
                Rectangle().fill(PaperPalette.border).frame(width: 1)
                    .accessibilityHidden(true)
                LedgerView(lines: lines, earlierInRecord: meeting.ledger.dropped > 0)
                    .frame(minWidth: 240, maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .layoutPriority(1.25)
                    .padding(.leading, 26)
            }
            .padding(.top, 14)
            .overlay(alignment: .top) {
                Rectangle().fill(PaperPalette.border).frame(height: 1).accessibilityHidden(true)
            }
            AskPanel(live: live, context: lines.filter { !$0.wet })
        }
        .padding(.horizontal, 36)
        .padding(.top, 26)
        .padding(.bottom, 22)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var header: some View {
        HStack(alignment: .lastTextBaseline, spacing: 16) {
            VStack(alignment: .leading, spacing: 6) {
                Text(meeting.title ?? meeting.appName.map { "\($0) call" } ?? "Live meeting")
                    .font(.system(.title, design: .serif, weight: .medium))
                    .foregroundStyle(Theme.text)
                    .accessibilityAddTraits(.isHeader)
                HStack(spacing: 10) {
                    status
                    if let app = meeting.appName, meeting.title != nil {
                        Text(app)
                    }
                    if let started = live.startedAt {
                        Text("started \(started.formatted(date: .omitted, time: .shortened))")
                    }
                    if let mic = micLine {
                        Text(mic)
                    }
                    ForEach(sideWarnings, id: \.self) { warning in
                        Text(warning).foregroundStyle(Theme.alert)
                    }
                }
                .font(Typography.timestamp)
                .foregroundStyle(Theme.secondaryText)
            }
            Spacer(minLength: 0)
            VStack(alignment: .trailing, spacing: 8) {
                if !meeting.stopping {
                    Button("Stop") { meetings.stop() }
                        .keyboardShortcut(".", modifiers: .command)
                        .accessibilityHint("Stops recording; the final pass then writes the record")
                }
                legend
            }
        }
    }

    /// Which mic, when the reason is worth saying ("why is it using the laptop mic?").
    private var micLine: String? {
        guard let name = meeting.micName else { return nil }
        switch meeting.micReason {
        case .builtInForBluetoothOutput?: return "\(name), because your headphones are Bluetooth"
        case .headsetMicSetting?: return "\(name), the headset's own mic"
        default: return nil
        }
    }

    /// Recording with its clock, or blotting once capture stopped.
    @ViewBuilder private var status: some View {
        if meeting.stopping {
            Label("Blotting…", systemImage: "drop")
                .labelStyle(.titleOnly)
        } else if let started = live.startedAt {
            TimelineView(.periodic(from: started, by: 1)) { context in
                HStack(spacing: 6) {
                    Circle().fill(PaperPalette.recording).frame(width: 8, height: 8)
                        .accessibilityHidden(true)
                    Text("Recording · \(liveClock(ms: live.elapsedMs(at: context.date)))")
                        .monospacedDigit()
                }
            }
        }
    }

    /// A side that stopped or delivers only silence, in words.
    private var sideWarnings: [String] {
        var out: [String] = []
        if meeting.sides[.far] == .zeros { out.append("The others' audio is silent") }
        if meeting.sides[.far] == .stopped { out.append("The others' audio stopped") }
        if meeting.sides[.mic] == .zeros { out.append("Your microphone is silent") }
        if meeting.sides[.mic] == .stopped { out.append("Your microphone stopped") }
        return out
    }

    private var legend: some View {
        HStack(spacing: 14) {
            legendItem(Theme.text, "you")
            legendItem(PaperPalette.them, "them")
            Text("grey = still settling")
        }
        .font(Typography.timestamp)
        .foregroundStyle(Theme.secondaryText)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Your lines are black, the others' are sepia; grey lines are still settling.")
    }

    private func legendItem(_ color: Color, _ label: String) -> some View {
        HStack(spacing: 6) {
            Circle().fill(color).frame(width: 8, height: 8)
            Text(label)
        }
    }

    private var notes: some View {
        VStack(alignment: .leading, spacing: 10) {
            Paper.Eyebrow(text: "Your notes")
            ZStack(alignment: .topLeading) {
                NotesEditor(
                    initialText: live.notesText,
                    onEdit: { text, paragraph in live.notesEdited(text, caretParagraph: paragraph) },
                    onLeave: { live.notesLeft() })
                if live.notesText.isEmpty {
                    Text("Type as you listen.")
                        .font(.system(size: 17, design: .serif))
                        .foregroundStyle(Theme.secondaryText)
                        .padding(.leading, 5)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            Text("Each line is kept with the moment you wrote it. After the call your notes lead the record, filled in from what was said.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .lineLimit(3)
        }
    }
}

/// What is being said: dry lines are settled, the wet line at the end is still settling.
struct LedgerView: View {
    let lines: [LiveLine]
    /// The oldest lines were let go of in memory (the record keeps every one).
    var earlierInRecord = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Paper.Eyebrow(text: "What's being said")
                .padding(.bottom, 6)
            if lines.isEmpty {
                Text("Waiting for someone to speak.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if earlierInRecord {
                            Text("Earlier lines are in the record.")
                                .font(Typography.caption)
                                .foregroundStyle(Theme.secondaryText)
                                .padding(.bottom, 6)
                        }
                        ForEach(lines) { line in
                            LedgerRow(line: line).id(line.id)
                        }
                    }
                }
                .onChange(of: lines.last) { _, last in
                    if let last { proxy.scrollTo(last.id, anchor: .bottom) }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("What's being said")
    }
}

private struct LedgerRow: View {
    let line: LiveLine

    private var mine: Bool { line.channel == .mic }
    private var color: Color { mine ? Theme.text : PaperPalette.them }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(line.atMs.map { liveClock(ms: $0) } ?? "")
                .font(Typography.timestamp)
                .foregroundStyle(Theme.secondaryText)
                .frame(width: 52, alignment: .trailing)
            Circle().fill(color).frame(width: 8, height: 8)
                .opacity(line.wet ? 0.6 : 1)
            VStack(alignment: .leading, spacing: 2) {
                Text(mine ? "You" : "Them")
                    .font(.system(.caption, weight: .semibold))
                    .foregroundStyle(mine ? Theme.text : PaperPalette.them)
                    .opacity(line.wet ? 0.8 : 1)
                Text(line.text)
                    .font(.system(size: 15.5, design: .serif))
                    .italic(line.wet)
                    .foregroundStyle(line.wet ? Theme.secondaryText : Theme.text)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 7)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilityText)
    }

    private var accessibilityText: String {
        let who = mine ? "You" : "Them"
        let when = line.atMs.map { " at \(liveClock(ms: $0))" } ?? ""
        return line.wet ? "\(who), still settling: \(line.text)" : "\(who)\(when): \(line.text)"
    }
}

/// Ask: the far end's questions (⌘1 to ⌘4 answers one) and a field for anything else.
struct AskPanel: View {
    @Bindable var live: LiveModel
    let context: [LiveLine]
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !live.stack.questions.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Paper.Eyebrow(text: "Asked of you")
                    ForEach(Array(live.stack.questions.enumerated()), id: \.element.id) { slot, question in
                        Button {
                            live.answerStacked(slot, context: context)
                        } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text("⌘\(slot + 1)")
                                    .font(Typography.timestamp)
                                    .foregroundStyle(slot == 0 ? Theme.accent : Theme.secondaryText)
                                Text(question.text)
                                    .foregroundStyle(slot == 0 ? Theme.text : Theme.secondaryText)
                                    .lineLimit(2)
                                    .multilineTextAlignment(.leading)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .keyboardShortcut(KeyEquivalent(Character("\(slot + 1)")), modifiers: .command)
                        .accessibilityLabel("Answer: \(question.text)")
                        .accessibilityHint("Command \(slot + 1)")
                    }
                }
            }
            HStack(spacing: 10) {
                TextField("Ask about this call", text: $live.askText)
                    .textFieldStyle(.plain)
                    .font(Typography.body)
                    .focused($fieldFocused)
                    .onSubmit { live.submitAsk(context: context) }
                    .accessibilityLabel("Ask about this call")
                Button("Focus Ask") { fieldFocused = true }
                    .keyboardShortcut("i", modifiers: .command)
                    .hidden()
                    .accessibilityHidden(true)
                Text("⌘ I")
                    .font(Typography.timestamp)
                    .foregroundStyle(Theme.secondaryText)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(PaperPalette.border))
                    .accessibilityHidden(true)
            }
            ForEach(live.asked.prefix(2)) { asked in
                VStack(alignment: .leading, spacing: 2) {
                    Text(asked.question)
                        .font(.system(.callout, weight: .semibold))
                        .foregroundStyle(Theme.text)
                    switch asked.answer {
                    case nil:
                        Text("Thinking…").foregroundStyle(Theme.secondaryText)
                    case .answer(let text):
                        // Model text: a verbatim Text (never parsed as markdown), and no link in
                        // it may open anything.
                        Text(verbatim: text).foregroundStyle(Theme.text).textSelection(.enabled)
                            .refusingLinks()
                    case .unavailable(let why):
                        Text(why).foregroundStyle(Theme.secondaryText)
                    }
                }
                .font(Typography.caption)
                .accessibilityElement(children: .combine)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .paperCard()
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Ask")
    }
}

/// The notes editor: an NSTextView on TextKit 2 (NSTextLayoutManager), plain text, New York.
/// Reports every edit with the caret's paragraph, and when the user moves to another paragraph or
/// leaves the editor, so the model saves each line once the user is done with it.
struct NotesEditor: NSViewRepresentable {
    /// The text it starts with (the notes typed before the screen was last left).
    let initialText: String
    let onEdit: (String, Int?) -> Void
    let onLeave: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onEdit: onEdit, onLeave: onLeave)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let textView = Self.makeTextView()
        textView.string = initialText
        textView.delegate = context.coordinator
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        scroll.documentView = textView
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.onEdit = onEdit
        context.coordinator.onLeave = onLeave
    }

    /// Takes whatever room it is offered and asks for none: the text scrolls inside. Without this
    /// SwiftUI sizes it from the text view's full height, and the window's minimum size (which the
    /// hosting controller follows) grows with every line typed.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 240, height: proposal.height ?? 160)
    }

    /// The text view, on TextKit 2.
    static func makeTextView() -> NSTextView {
        let textView = NSTextView(usingTextLayoutManager: true)
        textView.isRichText = false
        textView.importsGraphics = false
        textView.allowsUndo = true
        textView.drawsBackground = false
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.font = NSFont(
            descriptor: NSFont.systemFont(ofSize: 17).fontDescriptor.withDesign(.serif) ?? NSFont.systemFont(ofSize: 17).fontDescriptor,
            size: 17)
        // Ink on paper, paper on night paper: the text colour of Theme.text, as an NSColor.
        textView.textColor = NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? Palette.paper.nsColor : Palette.ink.nsColor
        }
        textView.insertionPointColor = Palette.sepia.nsColor
        textView.textContainerInset = NSSize(width: 0, height: 2)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.setAccessibilityLabel("Your notes")
        textView.setAccessibilityHelp("Type as you listen. Each line is saved with the moment you wrote it.")
        return textView
    }

    /// The paragraph the caret is in, counting from 0.
    static func paragraph(of location: Int, in text: String) -> Int {
        let ns = text as NSString
        let upTo = min(max(0, location), ns.length)
        return ns.substring(to: upTo).reduce(0) { $1 == "\n" ? $0 + 1 : $0 }
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var onEdit: (String, Int?) -> Void
        var onLeave: () -> Void
        private var lastParagraph: Int?

        init(onEdit: @escaping (String, Int?) -> Void, onLeave: @escaping () -> Void) {
            self.onEdit = onEdit
            self.onLeave = onLeave
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            report(textView)
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            let paragraph = NotesEditor.paragraph(of: textView.selectedRange().location, in: textView.string)
            // Moving to another line is when the line left is saved.
            if paragraph != lastParagraph {
                report(textView)
            }
        }

        func textDidEndEditing(_ notification: Notification) {
            lastParagraph = nil
            onLeave()
        }

        private func report(_ textView: NSTextView) {
            let paragraph = NotesEditor.paragraph(of: textView.selectedRange().location, in: textView.string)
            lastParagraph = paragraph
            onEdit(textView.string, paragraph)
        }
    }
}
