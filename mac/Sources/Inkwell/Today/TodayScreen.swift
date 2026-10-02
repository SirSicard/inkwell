// Today (the canvas's "Today"): over the orb, the date, a greeting, the status line and Record now;
// the live meeting's card while one records or blots; then what needs the user, the last meeting,
// what is up next on the calendar, what is owed soon, and today's and this week's counts, each a
// translucent card.
//
// Everything is read from the library (LibraryModel), the core's events (CoreStore), the
// permission cards and what is owed (ScreenModels, the Settings and Owed screens' models) and the
// calendar (UpNextModel). It is asked for when the screen appears and when the app comes back to
// the front; a finished meeting or a new dictation refreshes it through the library's events.
import AppKit
import InkBridge
import SwiftUI

struct TodayScreen: View {
    @Environment(CoreStore.self) private var store
    @Environment(LibraryModel.self) private var library
    @Environment(UpNextModel.self) private var upNext
    @Environment(Router.self) private var router
    @Environment(ScreenModels.self) private var screens
    @Environment(WindowPresence.self) private var presence
    @State private var showAllNeeds = false
    /// The content's width: two columns (the canvas's 1.3 : 1) from 624 pt, else one.
    @State private var width: CGFloat = 0
    /// The visible height: the counts sit at the foot of the screen, as the canvas has them.
    @State private var height: CGFloat = 0

    private var calendar: Calendar { library.calendar }

    var body: some View {
        let now = library.now()
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                hero(now)
                if let meeting = store.meeting {
                    LiveCard(meeting: meeting)
                }
                needsYou(now)
                if width >= 624 {
                    let left = (width - 18) * 1.3 / 2.3
                    HStack(alignment: .top, spacing: 18) {
                        card(lastMeeting(now)).frame(width: left, alignment: .leading)
                        VStack(alignment: .leading, spacing: 18) {
                            card(upNextSection(now))
                            card(owedSoon(now))
                        }
                        .frame(width: width - 18 - left, alignment: .leading)
                    }
                } else {
                    VStack(alignment: .leading, spacing: 18) {
                        card(lastMeeting(now))
                        card(upNextSection(now))
                        card(owedSoon(now))
                    }
                }
                Spacer(minLength: 0)
                stats
            }
            .padding(.horizontal, 36)
            .padding(.top, 20)
            .padding(.bottom, 28)
            .frame(maxWidth: .infinity, minHeight: height, alignment: .topLeading)
        }
        .scrollContentBackground(.hidden)
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
            width = max(size.width - 72, 0)
            height = size.height
        }
        .onAppear {
            library.refreshToday()
            screens.owed.load()
            // A screen showing permissions: checked now, and again whenever the app comes back to
            // the front while it is up (the user may be back from System Settings).
            screens.permissions.screenAppeared()
            upNext.refresh()
        }
        .onDisappear {
            screens.permissions.screenDisappeared()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            upNext.refresh()
        }
    }

    // MARK: Hero

    /// Over the orb: the date, the greeting and the status line on the left, Record now on the
    /// right, at the foot of the orb's space.
    private func hero(_ now: Date) -> some View {
        HStack(alignment: .bottom, spacing: 20) {
            VStack(alignment: .leading, spacing: 4) {
                Text(LibraryFormat.longDay(now, calendar: calendar))
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                Text(LibraryFormat.greeting(now, calendar: calendar))
                    .font(Typography.greeting)
                    .foregroundStyle(Theme.text)
                    .lineLimit(1)
                    .minimumScaleFactor(0.5)
                    .accessibilityAddTraits(.isHeader)
                Text(statusLine)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .padding(.top, 4)
                if let line = SpeechModels.todayLine(screens.catalogue.speech) {
                    SpeechModelLine(line: line)
                        .padding(.top, 6)
                }
            }
            Spacer(minLength: 0)
            RecordControls()
        }
        .padding(.horizontal, 14)
        .frame(minHeight: 240, alignment: .bottom)
    }

    /// "Listening for calls · Hold fn to dictate": the core's state and the key dictation uses now,
    /// unless no speech model could type what it hears (the line under it says so).
    private var statusLine: String {
        let key = DictationModel.key(screens.dictation.key)?.name ?? screens.dictation.key
        let listening = RecordControls.listeningText(recording: store.meeting != nil, listening: store.listening)
        let hold = screens.catalogue.speech.offersDictation ? "Hold \(key) to dictate" : ""
        return [listening, hold]
            .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
            .joined(separator: " · ")
    }

    /// A section on its card.
    private func card(_ content: some View) -> some View {
        content
            .padding(.vertical, 20)
            .padding(.horizontal, 22)
            .frame(maxWidth: .infinity, alignment: .leading)
            .paperCard()
    }

    // MARK: Needs you

    private func needItems(_ now: Date) -> [NeedsYouItem] {
        Self.needItems(library: library, store: store, permission: screens.permissions.state, now: now)
    }

    /// What the banner lists, from the models Today reads: the permission cards, the watchdog and
    /// notices (CoreStore), and the far-end check (LibraryModel), where "could not read" stays
    /// apart from "none".
    static func needItems(
        library: LibraryModel, store: CoreStore, permission: (PermissionCard) -> CardState, now: Date
    ) -> [NeedsYouItem] {
        NeedsYou.items(
            permission: permission, farEnd: library.farEnd,
            meeting: store.meeting, notices: store.notices, now: now, calendar: library.calendar)
    }

    @ViewBuilder
    private func needsYou(_ now: Date) -> some View {
        let items = needItems(now)
        if let first = items.first {
            VStack(alignment: .leading, spacing: 10) {
                NeedsYouRow(item: first, perform: perform)
                if items.count > 1 {
                    if showAllNeeds {
                        ForEach(items.dropFirst()) { item in
                            Divider().overlay(PaperPalette.border)
                            NeedsYouRow(item: item, perform: perform)
                        }
                    }
                    Button(showAllNeeds ? "Show less" : "\(items.count - 1) more") {
                        showAllNeeds.toggle()
                    }
                    .buttonStyle(.link)
                    .font(.callout)
                    .padding(.leading, 44)
                }
            }
            .padding(.vertical, 16)
            .padding(.horizontal, 20)
            .paperCard(alert: true)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Needs you")
        }
    }

    private func perform(_ action: NeedsYouItem.Action) {
        switch action {
        case .allow(let card):
            screens.permissions.request(card)
        case .dismiss(let id):
            store.dismissNotice(id)
        case .retryChecks:
            library.refreshToday()
        }
    }

    // MARK: Last meeting

    @ViewBuilder
    private func lastMeeting(_ now: Date) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Paper.Eyebrow(text: "Last meeting")
            if let meeting = library.lastMeeting {
                let record = meeting.record
                Button {
                    openRecord(record.record)
                } label: {
                    Text(LibraryFormat.title(of: record))
                        .font(.system(size: 30, design: .serif))
                        .foregroundStyle(Theme.text)
                        .multilineTextAlignment(.leading)
                        .lineLimit(3)
                }
                .buttonStyle(.plain)
                .accessibilityHint("Opens the record")
                Text(LibraryFormat.headerLine(record, people: meeting.people, now: now, calendar: calendar))
                    .font(PaperType.meta)
                    .foregroundStyle(Theme.secondaryText)
                // The summary's first paragraph; its headline is already the title above.
                if let lede = meeting.summary?.lede {
                    Text(lede)
                        .refusingLinks()
                        .font(PaperType.lede)
                        .lineSpacing(3)
                        .foregroundStyle(Theme.text)
                        .padding(.top, 4)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 10) {
                    Button("Open record") { openRecord(record.record) }
                        .buttonStyle(PaperButtonStyle())
                    if record.hasAudio {
                        let from = meeting.owed.compactMap(\.atMs).first ?? 0
                        Button {
                            openRecord(record.record, seekMs: from, play: true)
                        } label: {
                            HStack(spacing: 8) {
                                Image(systemName: "play.fill").font(.caption)
                                Text(from > 0 ? "Play from" : "Play")
                                if from > 0 {
                                    Text(LibraryFormat.stamp(ms: from)).font(PaperType.meta)
                                }
                            }
                        }
                        .buttonStyle(PaperButtonStyle())
                        .accessibilityLabel(from > 0 ? "Play from \(LibraryFormat.stamp(ms: from))" : "Play")
                    }
                }
                .padding(.top, 6)
            } else if library.lastMeetingLoad == .failed {
                // Never "no meetings": the library could not be read.
                Text("Couldn't load your last meeting.")
                    .font(PaperType.reading)
                    .foregroundStyle(Theme.secondaryText)
                Button("Try again") { library.refreshToday() }
                    .buttonStyle(.link)
            } else if library.lastMeetingLoad == .loaded {
                Text("No meetings yet. When you join a call, Inkwell records both sides and the record lands here.")
                    .font(PaperType.reading)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .contain)
    }

    private func openRecord(_ record: String, seekMs: Int64? = nil, play: Bool = false) {
        library.open(record, seekMs: seekMs, play: play)
        router.open(.library)
    }

    // MARK: Up next

    private func upNextSection(_ now: Date) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Paper.Eyebrow(text: "Up next")
            switch upNext.access {
            case .allowed:
                if let event = upNext.event {
                    Text(event.title)
                        .font(.headline)
                        .foregroundStyle(Theme.text)
                    // Redrawn on the minute so "in 42 min" stays true, and only while the window is
                    // on screen: hidden, covered or minimised, it draws once and stays still.
                    TimelineView(MinuteSchedule(paused: !upNext.ticks(onScreen: presence.onScreen))) { context in
                        Text(([LibraryFormat.time(event.start, calendar: calendar)] + [event.app].compactMap { $0 }
                            + [MeetingApp.startsIn(event.start, now: context.date)]).joined(separator: " · "))
                            .font(PaperType.meta)
                            .foregroundStyle(PaperPalette.quiet)
                    }
                    Text(event.app.map { "Records when \($0) opens the microphone" } ?? "Records when the call opens the microphone")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                } else {
                    Text("Nothing else on your calendar today.")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                }
            case .notAsked, .checking:
                Text("See your next meeting here.")
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
                Button("Show my next meeting") {
                    upNext.connect()
                }
                .buttonStyle(PaperButtonStyle())
                .accessibilityHint("Asks for access to your calendars")
            case .off, .unknown:
                Text("Calendar access is off, so Inkwell can't show your next meeting.")
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                // The same request as the Settings card's: here, System Settings at Calendars.
                Button("Open Calendar settings") { upNext.connect() }
                    .buttonStyle(.link)
            }
        }
        .accessibilityElement(children: .contain)
    }

    // MARK: Owed soon

    /// How many promises Today lists.
    static let owedShown = 3

    /// The Owed screen's list (OwedModel), soonest due first: its first three.
    private func owedSoon(_ now: Date) -> some View {
        let owed = screens.owed
        return VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Paper.Eyebrow(text: "Owed soon")
                Spacer()
                if !owed.items.isEmpty {
                    Button("All \(owed.items.count)") { router.open(.owed) }
                        .buttonStyle(.link)
                        .font(.callout)
                }
            }
            if owed.loaded && owed.items.isEmpty {
                Text("Nothing owed. Promises made in meetings show up here.")
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
            }
            ForEach(owed.items.prefix(Self.owedShown), id: \.id) { item in
                OwedSoonRow(
                    item: item, due: owed.due(item, now: now), calendar: calendar,
                    done: { owed.markDone(item.id) },
                    open: { ms in openRecord(item.record, seekMs: ms, play: true) })
            }
        }
        .accessibilityElement(children: .contain)
    }

    // MARK: Stats

    private var stats: some View {
        let dictated = library.today?.kinds.first { $0.kind == .dictation }
        let met = library.week?.kinds.first { $0.kind == .meeting }
        // A count that could not be read says so; it is never shown as zero.
        let lines = [
            dictated.map { "Dictated today · \($0.words.formatted()) words · \(LibraryFormat.duration(ms: $0.durationMs))" }
                ?? (library.todayLoad == .failed ? "Dictated today · couldn't be counted" : nil),
            met.map { "This week · \($0.records) \($0.records == 1 ? "meeting" : "meetings") · \(LibraryFormat.duration(ms: $0.durationMs))" }
                ?? (library.weekLoad == .failed ? "This week · couldn't be counted" : nil),
        ].compactMap { $0 }
        // Side by side when they fit, one under the other when not: never a line broken mid-count.
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: 28) {
                ForEach(lines, id: \.self) { Text($0).lineLimit(1) }
                Spacer(minLength: 0)
            }
            VStack(alignment: .leading, spacing: 4) {
                ForEach(lines, id: \.self) { Text($0) }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .font(PaperType.meta)
        .foregroundStyle(Theme.secondaryText)
        .padding(.top, 8)
        .padding(.horizontal, 14)
        .accessibilityElement(children: .combine)
    }
}

/// One thing that needs the user: an icon, what is wrong, and the one thing to do about it.
struct NeedsYouRow: View {
    let item: NeedsYouItem
    let perform: (NeedsYouItem.Action) -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            Image(systemName: "exclamationmark.circle")
                .font(.title2)
                .foregroundStyle(Theme.alert)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title)
                    .font(.headline)
                    .foregroundStyle(Theme.text)
                Text(item.detail)
                    .font(.callout)
                    .foregroundStyle(PaperPalette.quiet)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            Button(item.actionTitle) { perform(item.action) }
                .buttonStyle(PaperButtonStyle(prominent: true))
        }
        .accessibilityElement(children: .contain)
    }
}

/// One promise on Today: a circle to mark it done, what is owed, and when and where it was said.
struct OwedSoonRow: View {
    let item: OwedItem
    let due: DueLabel
    let calendar: Calendar
    let done: () -> Void
    let open: (Int64) -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Button(action: done) {
                Circle().strokeBorder(Theme.text, lineWidth: 1.5).frame(width: 20, height: 20)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Mark done: \(item.text)")
            VStack(alignment: .leading, spacing: 3) {
                Text(item.text)
                    .font(.body)
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 4) {
                    Text([due == .undated ? nil : due.text(calendar: calendar), item.recordTitle]
                        .compactMap { $0 }.joined(separator: " · "))
                        .lineLimit(1)
                    if let at = item.saidAtMs {
                        Button("▸ \(LibraryFormat.stamp(ms: at))") { open(at) }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Play where it was said, \(LibraryFormat.stamp(ms: at))")
                    }
                }
                .font(PaperType.meta)
                .foregroundStyle(due.isOverdue ? PaperPalette.alertText : Theme.secondaryText)
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// Under Today's greeting while no speech model is installed: what that means, and the download
/// of the recommended set (or how far it has got, or why it failed).
private struct SpeechModelLine: View {
    let line: String
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        let catalogue = screens.catalogue
        VStack(alignment: .leading, spacing: 8) {
            Text(line)
                .font(Typography.caption)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            switch catalogue.speechDownload {
            case .notStarted:
                HStack(spacing: 10) {
                    Button(catalogue.recommendedMB.map { "Download speech models (\($0) MB)" } ?? "Download speech models") {
                        catalogue.downloadRecommended()
                    }
                    .buttonStyle(PaperButtonStyle())
                    // Where the files come from, as the first run says it.
                    let hosts = CatalogueModel.sources(catalogue.models.filter { CatalogueModel.recommended.contains($0.id) })
                    if !hosts.isEmpty {
                        Text("From \(hosts)")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                    }
                }
            case .downloading(let percent):
                Text(percent.map { "Downloading speech models · \($0) %" } ?? "Downloading speech models…")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .monospacedDigit()
            case .failed(let why):
                HStack(spacing: 10) {
                    Text("The download failed: \(why)")
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                        .fixedSize(horizontal: false, vertical: true)
                    Button("Try again") { catalogue.downloadRecommended() }
                        .buttonStyle(PaperButtonStyle())
                }
            }
        }
        .accessibilityElement(children: .contain)
    }
}
