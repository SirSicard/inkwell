// Today (the canvas's "Today"): the date and a greeting, what needs the user, the last meeting,
// what is up next on the calendar, what is owed soon, and today's and this week's counts. The ink
// zone beside it is the shell's (InkRail); this is the content to its right.
//
// Everything is read from the library (LibraryModel), the core's events (CoreStore) and the
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
    @State private var showAllNeeds = false
    /// The user went to System Settings from here: check again when the app comes back.
    @State private var inSettings = false
    /// The content's width: two columns (the canvas's 1.3 : 1) from 624 pt, else one.
    @State private var width: CGFloat = 0
    /// The visible height: the counts sit at the foot of the screen, as the canvas has them.
    @State private var height: CGFloat = 0

    private var calendar: Calendar { library.calendar }

    var body: some View {
        let now = library.now()
        ScrollView {
            VStack(alignment: .leading, spacing: 26) {
                header(now)
                needsYou(now)
                if width >= 624 {
                    let left = (width - 40) * 1.3 / 2.3
                    HStack(alignment: .top, spacing: 40) {
                        lastMeeting(now).frame(width: left, alignment: .leading)
                        VStack(alignment: .leading, spacing: 26) {
                            upNextSection(now)
                            owedSoon(now)
                        }
                        .frame(width: width - 40 - left, alignment: .leading)
                    }
                } else {
                    VStack(alignment: .leading, spacing: 26) {
                        lastMeeting(now)
                        upNextSection(now)
                        owedSoon(now)
                    }
                }
                Spacer(minLength: 0)
                stats
            }
            .padding(.horizontal, 48)
            .padding(.top, 38)
            .padding(.bottom, 28)
            .frame(maxWidth: .infinity, minHeight: height, alignment: .topLeading)
        }
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
            width = max(size.width - 96, 0)
            height = size.height
        }
        .searchable(text: searchText, placement: .toolbar, prompt: "Search everything said")
        .onAppear {
            library.refreshToday()
            library.refreshPermissions(ifOlderThan: 300)
            upNext.refresh()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            // Back from System Settings, perhaps with a permission changed.
            if inSettings {
                inSettings = false
                library.refreshPermissions()
            }
            upNext.refresh()
        }
    }

    /// The toolbar's search is the Library's: typing on Today opens the Library's matches.
    private var searchText: Binding<String> {
        Binding(get: { library.query }, set: { words in
            library.query = words
            if !words.trimmingCharacters(in: .whitespaces).isEmpty { router.open(.library) }
        })
    }

    // MARK: Header

    private func header(_ now: Date) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionLabel(LibraryFormat.longDay(now, calendar: calendar))
            Text(LibraryFormat.greeting(now, calendar: calendar))
                .font(.system(.largeTitle, weight: .semibold))
                .foregroundStyle(Theme.text)
                .accessibilityAddTraits(.isHeader)
        }
    }

    // MARK: Needs you

    private func needItems(_ now: Date) -> [NeedsYouItem] {
        NeedsYou.items(
            permissions: library.permissions,
            farSilentMeetings: library.week?.farSilentMeetings ?? library.today?.farSilentMeetings ?? 0,
            farSilentSince: (library.week?.farSilentSinceUnixMs ?? library.today?.farSilentSinceUnixMs)
                .map(LibraryFormat.date(unixMs:)),
            meeting: store.meeting, notices: store.notices, now: now, calendar: calendar)
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
                            Divider().overlay(Paper.hairline)
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
            .padding(.horizontal, 18)
            .background(RoundedRectangle(cornerRadius: 14).fill(Paper.card))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Paper.sealTint, lineWidth: 1))
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Needs you")
        }
    }

    private func perform(_ action: NeedsYouItem.Action) {
        switch action {
        case .openSettings(let pane):
            if let url = pane.url {
                inSettings = true
                NSWorkspace.shared.open(url)
            }
        case .dismiss(let id):
            store.dismissNotice(id)
        }
    }

    // MARK: Last meeting

    @ViewBuilder
    private func lastMeeting(_ now: Date) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionLabel("Last meeting")
            if let meeting = library.lastMeeting {
                let record = meeting.record
                Button {
                    openRecord(record.record)
                } label: {
                    Text(LibraryFormat.title(of: record))
                        .font(.system(.title, design: .serif, weight: .medium))
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
            } else if library.lastMeetingLoaded {
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
            SectionLabel("Up next")
            switch upNext.access {
            case .granted:
                if let event = upNext.event {
                    Text(event.title)
                        .font(.headline)
                        .foregroundStyle(Theme.text)
                    // Redrawn once a minute while an event is shown, so "in 42 min" stays true.
                    TimelineView(.everyMinute) { context in
                        Text(([LibraryFormat.time(event.start, calendar: calendar)] + [event.app].compactMap { $0 }
                            + [MeetingApp.startsIn(event.start, now: context.date)]).joined(separator: " · "))
                            .font(PaperType.meta)
                            .foregroundStyle(Paper.quiet)
                    }
                    Text(event.app.map { "Records when \($0) opens the microphone" } ?? "Records when the call opens the microphone")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                } else {
                    Text("Nothing else on your calendar today.")
                        .font(.callout)
                        .foregroundStyle(Theme.secondaryText)
                }
            case .notDetermined:
                Text("See your next meeting here.")
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
                Button("Show my next meeting") {
                    Task { await upNext.connect() }
                }
                .buttonStyle(PaperButtonStyle())
                .accessibilityHint("Asks for access to your calendars")
            case .denied:
                Text("Calendar access is off, so Inkwell can't show your next meeting.")
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Open Calendar settings") {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars") {
                        NSWorkspace.shared.open(url)
                    }
                }
                .buttonStyle(.link)
            }
        }
        .accessibilityElement(children: .contain)
    }

    // MARK: Owed soon

    private func owedSoon(_ now: Date) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                SectionLabel("Owed soon")
                Spacer()
                if library.owedTotal > 0 {
                    Button("All \(library.owedTotal)") { router.open(.owed) }
                        .buttonStyle(.link)
                        .font(.callout)
                }
            }
            if library.owed.isEmpty {
                Text("Nothing owed. Promises made in meetings show up here.")
                    .font(.callout)
                    .foregroundStyle(Theme.secondaryText)
            }
            ForEach(library.owed, id: \.commitment.commitment) { item in
                OwedSoonRow(
                    item: item, now: now, calendar: calendar,
                    done: { library.setDone(item.commitment.commitment, true) },
                    open: { ms in openRecord(item.commitment.record, seekMs: ms, play: true) })
            }
        }
        .accessibilityElement(children: .contain)
    }

    // MARK: Stats

    private var stats: some View {
        let dictated = library.today?.kinds.first { $0.kind == .dictation }
        let met = library.week?.kinds.first { $0.kind == .meeting }
        let lines = [
            dictated.map { "Dictated today · \($0.words.formatted()) words · \(LibraryFormat.duration(ms: $0.durationMs))" },
            met.map { "This week · \($0.records) \($0.records == 1 ? "meeting" : "meetings") · \(LibraryFormat.duration(ms: $0.durationMs))" },
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
        .padding(.top, 16)
        .overlay(alignment: .top) { Rectangle().fill(Paper.hairline).frame(height: 1) }
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
                    .foregroundStyle(Paper.quiet)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            Button(item.actionTitle) { perform(item.action) }
                .buttonStyle(PaperButtonStyle(prominent: true))
        }
        .accessibilityElement(children: .contain)
    }
}

/// One commitment on Today: a circle to mark it done, what is owed, and when and where it was said.
struct OwedSoonRow: View {
    let item: OwedItem
    let now: Date
    let calendar: Calendar
    let done: () -> Void
    let open: (Int64) -> Void

    private var overdueDays: Int? {
        guard let due = item.commitment.dueAtUnixMs.map(LibraryFormat.date(unixMs:)), due < now else { return nil }
        return max(calendar.dateComponents([.day], from: calendar.startOfDay(for: due), to: calendar.startOfDay(for: now)).day ?? 0, 0)
    }

    private var when: String? {
        if let days = overdueDays {
            return days == 0 ? "Due today" : "\(days) \(days == 1 ? "day" : "days") overdue"
        }
        if let due = item.commitment.dueAtUnixMs.map(LibraryFormat.date(unixMs:)) {
            return LibraryFormat.day(due, now: now, calendar: calendar)
        }
        return item.commitment.due
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Button(action: done) {
                Circle().strokeBorder(Theme.text, lineWidth: 1.5).frame(width: 20, height: 20)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Mark done: \(item.commitment.text)")
            VStack(alignment: .leading, spacing: 3) {
                Text(item.commitment.text)
                    .font(.body)
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 4) {
                    Text([when, item.recordTitle].compactMap { $0 }.joined(separator: " · "))
                        .lineLimit(1)
                    if let at = item.commitment.provenance.map(\.startMs).min() {
                        Button("▸ \(LibraryFormat.stamp(ms: at))") { open(at) }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Play where it was said, \(LibraryFormat.stamp(ms: at))")
                    }
                }
                .font(PaperType.meta)
                .foregroundStyle(overdueDays != nil ? Paper.alert : Theme.secondaryText)
            }
        }
        .accessibilityElement(children: .contain)
    }
}
