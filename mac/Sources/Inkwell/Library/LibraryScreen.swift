// The Library (the canvas's "Library · a meeting record"): the records, newest first, filtered by
// kind or searched through everything said; beside them, the open record (RecordScreen).
import AppKit
import InkBridge
import SwiftUI

struct LibraryScreen: View {
    @Environment(LibraryModel.self) private var library

    var body: some View {
        HStack(spacing: 0) {
            LibraryColumn()
                .frame(width: 272)
                .background(PaperPalette.panel)
                .background(.ultraThinMaterial)
            Rectangle().fill(PaperPalette.border).frame(width: 1)
            Group {
                if library.selected != nil {
                    RecordScreen()
                } else {
                    VStack(spacing: 8) {
                        Text("Choose a record")
                            .font(.title3.weight(.semibold))
                            .foregroundStyle(Theme.text)
                        Text("Its notes, transcript and summary open here, with its audio.")
                            .font(.callout)
                            .foregroundStyle(Theme.secondaryText)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear {
            library.refreshList()
        }
    }
}

/// The list column: heading, kind filters, and the records or the search's matches.
struct LibraryColumn: View {
    @Environment(LibraryModel.self) private var library

    private var searching: Bool {
        !library.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text(searching ? "Search" : "Library")
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(Theme.text)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                Text("\(searching ? library.hits.count : library.records.count)\(library.hasMore && !searching ? "+" : "")")
                    .font(PaperType.meta)
                    .foregroundStyle(Theme.secondaryText)
                    .accessibilityLabel(searching ? "\(library.hits.count) matches" : "\(library.records.count) records")
            }
            .padding(.horizontal, 6)
            if let note = library.deletionNote {
                Text(note)
                    .font(PaperType.meta)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 6)
            }
            if !searching {
                KindFilter()
                    .padding(.horizontal, 6)
            }
            if searching {
                SearchResults()
            } else {
                RecordList()
            }
        }
        .padding(.vertical, 18)
        .padding(.horizontal, 12)
    }
}

/// All, Meetings, Dictations, Files: one at a time. All is where the Library opens.
struct KindFilter: View {
    @Environment(LibraryModel.self) private var library

    private static let kinds: [(RecordKind?, String)] = [
        (nil, "All"), (.meeting, "Meetings"), (.dictation, "Dictations"), (.fileImport, "Files"),
    ]

    var body: some View {
        // A chip's label never wraps ("Meetin/gs"): the four fit the 272 pt list column at this
        // padding (about 228 pt of its 236), and should they not, the row scrolls sideways.
        ViewThatFits(in: .horizontal) {
            chips
            ScrollView(.horizontal, showsIndicators: false) { chips }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Show")
    }

    private var chips: some View {
        HStack(spacing: 4) {
            ForEach(Self.kinds, id: \.1) { kind, title in
                let on = library.filter == kind
                Button {
                    library.filter = kind
                } label: {
                    Text(title)
                        .font(.system(size: Glow.Size.caption))
                        .lineLimit(1)
                        .fixedSize()
                        .padding(.horizontal, 7)
                        .frame(minHeight: 28)
                        .foregroundStyle(on ? Theme.buttonLabel : Theme.text)
                        .background(Capsule().fill(on ? Theme.buttonFill : PaperPalette.chip))
                        .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? .isSelected : [])
                .accessibilityHint(kind == nil ? "Shows every kind" : "Shows only \(title.lowercased())")
            }
        }
    }
}

/// The records, newest first. A native List: arrow keys move the selection and VoiceOver reads
/// each row.
struct RecordList: View {
    @Environment(LibraryModel.self) private var library
    @FocusState private var listFocused: Bool

    private var emptyText: (String, String) {
        switch library.filter {
        case .meeting?: ("No meetings yet", "When you join a call, Inkwell records both sides and the record lands here.")
        case .dictation?: ("No dictations yet", "Each dictation lands here, with the words it typed.")
        case .fileImport?: ("No imported files yet", "Audio and video files you import land here, transcribed.")
        case nil: ("Nothing here yet", "Meetings, dictations and imported files land here.")
        }
    }

    var body: some View {
        let now = library.now()
        if library.listLoad == .failed {
            // Never an empty library: the list could not be read.
            VStack(alignment: .leading, spacing: 6) {
                Text("Couldn't load the library").font(.headline).foregroundStyle(Theme.text)
                Button("Try again") { library.refreshList() }
                    .buttonStyle(.link)
            }
            .padding(.horizontal, 6)
            Spacer()
        } else if library.listLoad == .loaded && library.records.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text(emptyText.0).font(.headline).foregroundStyle(Theme.text)
                Text(emptyText.1).font(.callout).foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 6)
            Spacer()
        } else {
            List(selection: Binding(get: { library.selected }, set: { if let id = $0 { library.open(id) } })) {
                ForEach(library.records, id: \.record) { record in
                    RecordRowView(
                        record: record, now: now, calendar: library.calendar, selected: library.selected == record.record,
                        listFocused: listFocused)
                        .tag(record.record)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                        .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
                }
                if library.hasMore {
                    Button(library.moreLoad == .failed ? "Couldn't load older records. Try again" : "Show older") {
                        library.loadMore()
                    }
                    .buttonStyle(.link)
                    .disabled(library.moreLoad == .loading)
                    .listRowBackground(Color.clear)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .focused($listFocused)
            .accessibilityLabel("Records")
        }
    }
}

/// One record in the list: what it is called, and when and how long. Selected, it is a card; while
/// the list has the keyboard AppKit fills it with the accent instead, and its words are in the
/// button label.
struct RecordRowView: View {
    let record: RecordRow
    let now: Date
    let calendar: Calendar
    let selected: Bool
    let listFocused: Bool
    @Environment(\.controlActiveState) private var active

    private var onAccent: Bool { accentFillsRow(selected: selected, listFocused: listFocused, active: active) }
    private var card: Bool { selected && !onAccent }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(LibraryFormat.title(of: record))
                .font(.body.weight(.semibold))
                .foregroundStyle(onAccent ? Theme.onAccent : Theme.text)
                .lineLimit(2)
            Text(LibraryFormat.listLine(record, now: now, calendar: calendar))
                .font(PaperType.meta)
                .foregroundStyle(onAccent ? Theme.onAccent : Theme.secondaryText)
        }
        .padding(.vertical, 9)
        .padding(.horizontal, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(card ? PaperPalette.card : Color.clear))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(card ? PaperPalette.border : Color.clear, lineWidth: 1))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// Matches for the search, best first: the record, the words matched, and where.
struct SearchResults: View {
    @Environment(LibraryModel.self) private var library

    var body: some View {
        let now = library.now()
        if library.hits.isEmpty {
            Group {
                switch library.searchLoad {
                case .failed:
                    Text("Couldn't search the library.")
                case .loaded:
                    Text("Nothing said matches “\(library.hitsQuery)”.")
                case .idle, .loading:
                    Text("Searching…")
                }
            }
            .font(.callout)
            .foregroundStyle(Theme.secondaryText)
            .padding(.horizontal, 6)
            Spacer()
        } else {
            List {
                ForEach(Array(library.hits.enumerated()), id: \.offset) { _, hit in
                    Button {
                        library.open(hit.record, seekMs: hit.startMs)
                    } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            // A match in a record with no title (a dictation) is called by its words, as the
                            // list calls it, never "Untitled record"; the words aren't then repeated below.
                            Text(LibraryFormat.hitTitle(hit))
                                .font(.body.weight(.semibold))
                                .foregroundStyle(Theme.text)
                                .lineLimit(hit.title == nil ? 2 : 1)
                            if hit.title != nil {
                                Text(hit.snippet)
                                    .font(.system(.callout, design: .serif))
                                    .foregroundStyle(PaperPalette.quiet)
                                    .lineLimit(2)
                            }
                            Text("\(LibraryFormat.day(LibraryFormat.date(unixMs: hit.startedAtUnixMs), now: now, calendar: library.calendar)) · \(LibraryFormat.stamp(ms: hit.startMs))")
                                .font(PaperType.meta)
                                .foregroundStyle(Theme.secondaryText)
                        }
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(Color.clear)
                    .accessibilityHint("Opens the record at this moment")
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .accessibilityLabel("Matches")
        }
    }
}
