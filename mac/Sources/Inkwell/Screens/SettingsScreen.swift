// Settings: General (open at login, updates, Inkwell 0.2's history), Appearance, permissions with
// their live state, sound (the microphone and its test: SoundSection), dictation's keys, modes, snippets and voice commands (PhrasesSections), AI (the
// language model you bring, local-only mode, polish, summaries and Ask), meetings, stats
// (milestones and the typing speed), models (with measured accuracy, and Download for those not on
// this Mac), storage, and About with every notice the app ships. Each section is a card, as
// Today's are, with its heading inside.
import AppleEngines
import InkBridge
import SwiftUI

enum SettingsSection: String, CaseIterable, Identifiable {
    case general
    case appearance
    case permissions
    case sound
    case dictation
    case modes
    case snippets
    case voiceCommands
    case ai
    case meetings
    case stats
    case models
    case storage
    case about

    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: "General"
        case .appearance: "Appearance"
        case .permissions: "Permissions"
        case .sound: "Sound"
        case .dictation: "Dictation"
        case .modes: "Modes"
        case .snippets: "Snippets"
        case .voiceCommands: "Voice commands"
        case .ai: "AI"
        case .meetings: "Meetings"
        case .stats: "Stats"
        case .models: "Models"
        case .storage: "Storage"
        case .about: "About"
        }
    }
}

struct SettingsScreen: View {
    @Environment(ScreenModels.self) private var screens
    @State private var section: SettingsSection? = .general
    /// The section list has the keyboard: its selected row is drawn in the accent (`onAccent`).
    @FocusState private var sectionsFocused: Bool
    /// The section a click scrolled to: it stays selected while any of it is in view, as the last
    /// sections cannot scroll to the top.
    @State private var clicked: SettingsSection?
    /// The selection the scrolling made, which must not scroll the page again.
    @State private var followed: SettingsSection?

    var body: some View {
        HStack(spacing: 0) {
            List(SettingsSection.allCases, selection: $section) { item in
                Text(item.title).tag(item).onAccent(selected: section == item, listFocused: sectionsFocused)
            }
            .listStyle(.sidebar)
            .focused($sectionsFocused)
            .scrollContentBackground(.hidden)
            .frame(width: 188)
            .accessibilityLabel("Settings sections")
            Rectangle().fill(PaperPalette.border).frame(width: 1).accessibilityHidden(true)
            ScrollViewReader { proxy in
                ScrollView {
                    // Each section on its own card, as Today's are, apart as Today's are: in Dark,
                    // sections divided by hairlines alone ran together.
                    VStack(alignment: .leading, spacing: TodayColumnsLayout.spacing) {
                        GeneralSection(screens: screens).settingsCard(.general)
                        AppearanceSection(theme: screens.theme).settingsCard(.appearance)
                        PermissionsSection(permissions: screens.permissions).settingsCard(.permissions)
                        SoundSection(sound: screens.sound).settingsCard(.sound)
                        DictationSection(screens: screens, dictation: screens.dictation, permissions: screens.permissions)
                            .settingsCard(.dictation)
                        ModesSection(modes: screens.modes).settingsCard(.modes)
                        SnippetsSection(snippets: screens.snippets).settingsCard(.snippets)
                        VoiceCommandsSection(commands: screens.voiceCommands).settingsCard(.voiceCommands)
                        AISection(polish: screens.polish, screens: screens, cloud: screens.cloud).settingsCard(.ai)
                        MeetingsSection(permissions: screens.permissions, meetings: screens.meetings)
                            .settingsCard(.meetings)
                        StatsSettingsSection(stats: screens.stats).settingsCard(.stats)
                        ModelsSection(catalogue: screens.catalogue).settingsCard(.models)
                        StorageSection(storage: screens.storage, meetings: screens.meetings).settingsCard(.storage)
                        AboutSection().settingsCard(.about)
                    }
                    // The sections are the scroll's targets, for the list to follow (below).
                    .scrollTargetLayout()
                    .frame(maxWidth: Self.maxCardWidth, alignment: .leading)
                    .modifier(SettingsMargins())
                    .padding(.vertical, 28)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .scrollContentBackground(.hidden)
                .onChange(of: section) { _, section in
                    guard let section else { return }
                    if section == followed {
                        followed = nil
                        return
                    }
                    clicked = section
                    proxy.scrollTo(section, anchor: .top)
                }
                // The list follows the scrolling: the topmost section in view, and at the end of
                // the page the last one.
                .onScrollTargetVisibilityChange(idType: SettingsSection.self, threshold: 0.01) { visible in
                    if let clicked, visible.contains(clicked) { return }
                    clicked = nil
                    follow(SettingsSection.allCases.first(where: visible.contains))
                }
                .onScrollGeometryChange(for: Bool.self) { geometry in
                    geometry.contentOffset.y + geometry.containerSize.height >= geometry.contentSize.height - 1
                } action: { _, atEnd in
                    if atEnd, clicked == nil { follow(SettingsSection.allCases.last) }
                }
            }
        }
        .onAppear {
            screens.permissions.screenAppeared()
            screens.modes.load()
            screens.polish.load()
            screens.meetingsConsent.load()
            screens.dictation.load()
            screens.snippets.load()
            screens.voiceCommands.load()
            screens.importNote.load()
            screens.import02.check()
            screens.cloud.load()
            screens.catalogue.requery()
            screens.storage.measure()
            screens.sound.load()
        }
        .onDisappear { screens.permissions.screenDisappeared() }
    }

    /// The cards' widest: 760 pt of section inside their padding, as wide as the page was before.
    static let maxCardWidth: CGFloat = 760 + 2 * SectionCard.horizontal

    /// Selects `next` for the scrolling, without scrolling.
    private func follow(_ next: SettingsSection?) {
        guard let next, next != section else { return }
        followed = next
        section = next
    }
}

/// The page's margins either side: 28 pt, and 24 in a column too narrow to spare them (the window
/// at its smallest), so the sections keep the room their controls need there. Between the two the
/// margin grows with the column, so the sections' width only ever grows as the window widens (a
/// step would narrow them for a moment, and flip a picker from segments to a menu and back). 28,
/// not the 40 the page had before its cards: the cards' padding is inside it, and with 40 the voice
/// command form stacked in the default 1040-pt window (500 pt in its card, under its 520).
private struct SettingsMargins: ViewModifier {
    func body(content: Content) -> some View {
        SettingsMarginsLayout { content }
    }
}

struct SettingsMarginsLayout: Layout {
    static let wide: CGFloat = 28
    static let narrow: CGFloat = 24
    /// The column's width, margins included, up to which the margins are narrow, and from which
    /// they are wide.
    static let narrowUpTo: CGFloat = 400
    static let wideFrom: CGFloat = 440

    static func margin(_ width: CGFloat?) -> CGFloat {
        guard let width, width.isFinite else { return wide }
        let progress = min(max((width - narrowUpTo) / (wideFrom - narrowUpTo), 0), 1)
        return narrow + (wide - narrow) * progress
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let margin = Self.margin(proposal.width)
        let inner = ProposedViewSize(
            width: proposal.width.map { $0.isFinite ? max(0, $0 - 2 * margin) : $0 }, height: proposal.height)
        let size = subviews.reduce(CGSize.zero) { size, subview in
            let fitted = subview.sizeThatFits(inner)
            return CGSize(width: max(size.width, fitted.width), height: max(size.height, fitted.height))
        }
        return CGSize(width: size.width + 2 * margin, height: size.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        // The margin for the width proposed, as measured; the bounds only place it.
        let margin = Self.margin(proposal.width)
        let inner = ProposedViewSize(
            width: proposal.width.map { $0.isFinite ? max(0, $0 - 2 * margin) : $0 }, height: proposal.height)
        for subview in subviews {
            subview.place(at: CGPoint(x: bounds.minX + margin, y: bounds.minY), anchor: .topLeading, proposal: inner)
        }
    }
}

extension View {
    /// A Settings section on its card (Today's, sectionCard), its heading inside: one VoiceOver
    /// group named for the section, and the target the list scrolls to, so it lands on the card.
    func settingsCard(_ section: SettingsSection) -> some View {
        transformAnchorPreference(key: SettingsCardBounds.self, value: .bounds) {
            $0.append(SettingsCardBounds.Part(section: section, isCard: false, bounds: $1))
        }
        .sectionCard()
        .transformAnchorPreference(key: SettingsCardBounds.self, value: .bounds) {
            $0.append(SettingsCardBounds.Part(section: section, isCard: true, bounds: $1))
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(section.title)
        .id(section)
    }

    /// A group inside a section's card (the permission rows, Inkwell 0.2's import): a hairline
    /// round it and no card of its own, whose material would lie over the card's.
    func cardGroup() -> some View {
        modifier(CardGroup())
    }
}

/// Where each Settings card is, and its section inside it: anchors, which change no layout, read by
/// the layout tests (SettingsCardsLayoutTests) and by no view of the app's.
struct SettingsCardBounds: PreferenceKey {
    struct Part {
        let section: SettingsSection
        /// The card, or the section inside its padding.
        let isCard: Bool
        let bounds: Anchor<CGRect>
    }

    static var defaultValue: [Part] { [] }

    static func reduce(value: inout [Part], nextValue: () -> [Part]) {
        value.append(contentsOf: nextValue())
    }
}

/// cardGroup's hairline: the stronger border a card has under Increase Contrast or Reduce
/// Transparency (GlowCard), so the group keeps its edge there as the card it replaced did.
struct CardGroup: ViewModifier {
    /// Inside the card's 22 pt corner, a smaller one.
    static let radius: CGFloat = 14
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        let solid = reduceTransparency || contrast == .increased
        let shape = RoundedRectangle(cornerRadius: Self.radius, style: .continuous)
        content
            .clipShape(shape)
            .overlay(shape.strokeBorder(solid ? Theme.text.opacity(0.35) : PaperPalette.border, lineWidth: 1))
    }
}

/// A section's heading.
struct SectionTitle: View {
    let text: String
    var note: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(text)
                .font(Typography.heading)
                .foregroundStyle(Theme.text)
                .accessibilityAddTraits(.isHeader)
            if let note {
                Text(note).font(Typography.caption).foregroundStyle(Theme.secondaryText)
            }
        }
    }
}

// MARK: - Permissions

struct PermissionsSection: View {
    let permissions: PermissionsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionTitle(text: "Permissions")
            PermissionCards(permissions: permissions, onCard: true)
        }
    }
}

/// The four cards, in one frame: a card of its own in the first run, a group inside the
/// Permissions card in Settings (`onCard`).
struct PermissionCards: View {
    let permissions: PermissionsModel
    var onCard = false

    var body: some View {
        let rows = VStack(spacing: 0) {
            ForEach(PermissionCard.allCases) { card in
                PermissionRow(card: card, state: permissions.state(card)) { permissions.request(card) }
                if card != PermissionCard.allCases.last {
                    Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
                }
            }
        }
        if onCard {
            rows.cardGroup()
        } else {
            rows
                .clipShape(RoundedRectangle(cornerRadius: Glow.Radius.card, style: .continuous))
                .paperCard()
        }
    }
}

private struct PermissionRow: View {
    let card: PermissionCard
    let state: CardState
    let request: () -> Void

    /// The least room the words get beside the state: narrower, the state goes under them (the
    /// Permissions card in the window at its smallest, where "Checking…" broke).
    static let wordsMinimum: CGFloat = 150
    /// The icon's width, and the room between it and the words.
    private static let iconWidth: CGFloat = 22
    private static let gap: CGFloat = 14

    var body: some View {
        // Chosen from the ideal widths, the words' set to their minimum: no width is measured.
        ViewThatFits(in: .horizontal) {
            HStack(spacing: Self.gap) {
                icon.frame(width: Self.iconWidth, height: Self.iconWidth).accessibilityHidden(true)
                words.frame(minWidth: Self.wordsMinimum, idealWidth: Self.wordsMinimum, maxWidth: .infinity, alignment: .leading)
                trailing.fixedSize()
            }
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: Self.gap) {
                    icon.frame(width: Self.iconWidth, height: Self.iconWidth).accessibilityHidden(true)
                    words.frame(maxWidth: .infinity, alignment: .leading)
                }
                trailing.fixedSize().padding(.leading, Self.iconWidth + Self.gap)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 13)
        .background(state.isAlert ? PaperPalette.alertCard : Color.clear)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(card.title): \(spoken)")
        .accessibilityAction(named: actionTitle ?? "Check") { request() }
    }

    private var words: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(card.title).font(.system(.body, weight: .semibold)).foregroundStyle(Theme.text)
            Text(state.isAlert ? card.offDetail : card.detail)
                .font(Typography.caption)
                .foregroundStyle(state.isAlert ? Theme.text : Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var icon: some View {
        switch state {
        case .allowed:
            Image(systemName: "checkmark.circle.fill").font(.title2).foregroundStyle(Theme.text)
        case .off:
            Circle().strokeBorder(Theme.alert, style: StrokeStyle(lineWidth: 1.8, dash: [3, 2.4]))
        case .notAsked, .unknown:
            Circle().strokeBorder(Theme.secondaryText, lineWidth: 1.5)
        case .checking:
            ProgressView().controlSize(.small)
        }
    }

    private var actionTitle: String? {
        switch state {
        case .off: "Allow"
        case .notAsked: "Allow"
        case .unknown: "Open Settings"
        case .allowed, .checking: nil
        }
    }

    @ViewBuilder private var trailing: some View {
        if let actionTitle {
            if state == .off {
                Button(actionTitle, action: request).buttonStyle(.borderedProminent).tint(Theme.text)
            } else {
                Button(actionTitle, action: request)
            }
        } else {
            // A state in words, in the caption's face: mono is for timestamps and versions.
            Text(state == .allowed ? "Allowed" : "Checking…")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
    }

    private var spoken: String {
        switch state {
        case .allowed: "allowed"
        case .off: "off. \(card.offDetail)"
        case .notAsked: "not allowed yet"
        case .unknown: "can't be checked"
        case .checking: "checking"
        }
    }
}

// MARK: - Dictation

/// The dictation key and the voice-edit key, each held by the core: a change rebinds it at once.
private struct DictationSection: View {
    let screens: ScreenModels
    let dictation: DictationModel
    let permissions: PermissionsModel
    /// Each row's picker and key cap, as wide as they are: both rows give them the wider width, so
    /// the two Record a shortcut buttons start in one column whatever the keys are.
    @State private var dictateKeysWidth: CGFloat = 0
    @State private var editKeysWidth: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Dictation")
            SettingColumns {
                Text("Dictation")
            } controls: {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Toggle("Dictation", isOn: Binding(get: { dictation.isOn }, set: { dictation.setOn($0) }))
                        .toggleStyle(.switch)
                        .labelsHidden()
                        // Turned on mid-recording, the core would hold the keys the recorder listens for.
                        .disabled(shortcuts.recording != nil)
                    Text(dictation.isOn ? "The keys below are Inkwell's" : "Off: the keys do what they did before")
                        .foregroundStyle(Theme.secondaryText)
                }
            }
            .font(Typography.body)
            .padding(.vertical, 5)
            .accessibilityElement(children: .contain)
            SettingRow(title: "Dictate") {
                KeyControls {
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        Picker("Dictate", selection: Binding(get: { dictation.key }, set: { dictation.setKey($0) })) {
                            ForEach(DictationModel.keys) { key in
                                Text(key.name).tag(key.token)
                            }
                            // A recorded key is not a quick pick: it is listed so the picker shows it.
                            if !DictationModel.keys.contains(where: { $0.token == dictation.key }),
                               let recorded = DictationModel.key(dictation.key) {
                                Text(recorded.cap).accessibilityLabel(recorded.name).tag(dictation.key)
                            }
                        }
                        .labelsHidden()
                        .fixedSize()
                        .disabled(shortcuts.recording != nil)
                        Key(text: DictationModel.cap(dictation.key))
                            .accessibilityLabel(DictationModel.key(dictation.key)?.name ?? dictation.key)
                    }
                    .fixedSize()
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { dictateKeysWidth = $0 }
                    .frame(minWidth: keysColumn, alignment: .leading)
                } record: {
                    RecordShortcutButton(recorder: shortcuts, target: .dictation, what: "the dictation key")
                }
                KeyHint(text: "hold, speak, let go")
                ShortcutMessage(recorder: shortcuts, target: .dictation)
            }
            SettingRow(title: "Edit a selection") {
                KeyControls {
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        Picker("Edit a selection", selection: Binding(
                            get: { dictation.editKey ?? "off" },
                            set: { screens.chooseEditKey($0 == "off" ? nil : $0) }
                        )) {
                            Text("Off").tag("off")
                            ForEach(DictationModel.keys.filter { $0.token != dictation.key }) { key in
                                Text(key.name).tag(key.token)
                            }
                            if let edit = dictation.editKey, !DictationModel.keys.contains(where: { $0.token == edit }),
                               let recorded = DictationModel.key(edit) {
                                Text(recorded.cap).accessibilityLabel(recorded.name).tag(edit)
                            }
                        }
                        .labelsHidden()
                        .fixedSize()
                        .disabled(shortcuts.recording != nil)
                        if let edit = dictation.editKey, dictation.editKeyProblem == nil {
                            Key(text: DictationModel.cap(edit))
                                .accessibilityLabel(DictationModel.key(edit)?.name ?? edit)
                        }
                    }
                    .fixedSize()
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { editKeysWidth = $0 }
                    .frame(minWidth: keysColumn, alignment: .leading)
                } record: {
                    RecordShortcutButton(recorder: shortcuts, target: .edit, what: "the edit key")
                }
                KeyHint(text: "select text, hold, say what to change")
                ShortcutMessage(recorder: shortcuts, target: .edit)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(dictation.keyFailure ?? dictation.status)
                    .foregroundStyle(dictation.isProblem ? Theme.alert : Theme.secondaryText)
                if case .off(.needsAccessibility, _) = dictation.state {
                    Button("Allow \u{201C}Type for you\u{201D}") { permissions.request(.typeForYou) }
                } else if dictation.canRetry {
                    Button(dictation.retryTitle) { dictation.retry() }
                }
                if let problem = dictation.editKeyProblem {
                    Text(problem == DictationModel.editKeyLostText ? problem : "The edit key isn't held: \(problem)")
                        .foregroundStyle(Theme.alert)
                }
                if let problem = screens.editConsent.problem {
                    Text(problem).foregroundStyle(Theme.alert)
                }
                if let problem = dictation.settingsProblem {
                    Text("Dictation \(problem), so it uses the defaults for them.").foregroundStyle(Theme.alert)
                }
            }
            .font(Typography.caption)
            .fixedSize(horizontal: false, vertical: true)
            ImportKeyNoteView(model: screens.importNote, currentKey: DictationModel.key(dictation.key)?.name ?? dictation.key)
            Text("Editing sends the selection and what you say to a language model, and replaces the selection with the answer, so choosing its key asks you first where that is. Edits are not saved in the Library.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
        .consentStep(screens.editConsent, host: .settings)
        .shortcutRecording(shortcuts)
    }

    private var shortcuts: ShortcutRecorderModel { screens.shortcuts }
    private var keysColumn: CGFloat { max(dictateKeysWidth, editKeysWidth) }
}

/// A key row's controls: the picker and its cap, then Record a shortcut…, on one line where the
/// row has room for the button's longest label, else the button under them. Measured with that
/// label, both rows choose alike (their key slots are as wide) and the buttons stay in one column,
/// and pressing the button never moves it to the next line. Crossing the width rebuilds the
/// controls, so an open picker menu closes; a recording, held by the recorder, goes on.
struct KeyControls<Keys: View, Record: View>: View {
    @ViewBuilder var keys: Keys
    @ViewBuilder var record: Record

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                keys
                ZStack(alignment: .leading) {
                    // Only its width: never drawn, pressed, tabbed to or read out.
                    Button(RecordShortcutButton.recordingTitle) {}
                        .hidden()
                        .disabled(true)
                        .accessibilityHidden(true)
                    record
                }
            }
            VStack(alignment: .leading, spacing: 6) {
                keys
                record
            }
        }
    }
}

/// What a key does, under its controls, as the other rows' details are.
private struct KeyHint: View {
    let text: String

    var body: some View {
        Text(text)
            .font(Typography.caption)
            .foregroundStyle(Theme.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// "Record a shortcut…", and while recording, what to do and how to stop.
struct RecordShortcutButton: View {
    let recorder: ShortcutRecorderModel
    let target: ShortcutRecorderModel.Target
    /// "the dictation key", for VoiceOver.
    let what: String

    /// The title while recording, the longest it has.
    static let recordingTitle = "Press the keys\u{2026} (Esc cancels)"

    private var isRecording: Bool { recorder.recording == target }
    private var isChecking: Bool { recorder.checking?.target == target }

    var body: some View {
        // Pressed while recording or checking, it cancels: a check the core never answers is
        // given up after a few seconds anyway (ShortcutRecorderModel.checkTimeout).
        Button(isRecording ? Self.recordingTitle : isChecking ? "Cancel" : "Record a shortcut\u{2026}") {
            recorder.toggle(target)
        }
        .accessibilityLabel(isRecording
            ? "Recording a shortcut for \(what)"
            : isChecking ? "Cancel checking the shortcut for \(what)" : "Record a shortcut for \(what)")
        .accessibilityHint(isRecording
            ? "Press the keys you want: a right-hand modifier alone, a function key, or modifiers and a key. Escape on its own cancels."
            : isChecking ? "" : "Then press the keys you want to use.")
    }
}

/// What became of the last recording of a key: why it was refused, or a clash with a shortcut the
/// app knows, under the key's hint. VoiceOver hears it from the recorder when it happens, not each
/// time this appears.
private struct ShortcutMessage: View {
    let recorder: ShortcutRecorderModel
    let target: ShortcutRecorderModel.Target

    var body: some View {
        if let message = recorder.message(for: target) {
            Text(message.text)
                .font(Typography.caption)
                .foregroundStyle(message.isProblem ? Theme.alert : Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct Key: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(.callout, design: .monospaced))
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(PaperPalette.card, in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(PaperPalette.border))
    }
}

// MARK: - Modes

private struct ModesSection: View {
    let modes: ModesModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Modes", note: "Picked by the app you're typing in")
            if modes.failed {
                Text("Your modes could not be read.").foregroundStyle(Theme.alert)
            }
            ForEach(modes.rows) { row in
                SettingColumns {
                    Text(row.isDefault && modes.rows.count > 1 ? "Everywhere else" : row.name)
                        .font(.system(.body, weight: .semibold))
                } controls: {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 6) {
                            ForEach(row.traits, id: \.self) { Paper.Chip(text: $0) }
                        }
                        if row.apps.isEmpty {
                            Text(row.isDefault ? "Every app without a mode of its own" : "No apps")
                                .font(Typography.caption).foregroundStyle(Theme.secondaryText)
                        } else {
                            HStack(spacing: 6) {
                                ForEach(row.apps) { app in
                                    AppIcon(app: app)
                                }
                                Text(row.apps.map(\.name).joined(separator: ", "))
                                    .font(Typography.caption).foregroundStyle(Theme.secondaryText)
                                    .lineLimit(2)
                            }
                        }
                    }
                }
                .padding(.vertical, 10)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(accessibility(row))
                Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
            }
        }
    }

    private func accessibility(_ row: ModeRow) -> String {
        let apps = row.apps.isEmpty ? (row.isDefault ? "the default" : "no apps") : row.apps.map(\.name).joined(separator: ", ")
        return "\(row.name): \(row.traits.joined(separator: ", ")); used in \(apps)"
    }
}

private struct AppIcon: View {
    let app: AppLabel

    var body: some View {
        Group {
            if let icon = app.icon {
                Image(nsImage: icon).resizable()
            } else {
                Text(String(app.name.prefix(1)))
                    .font(.system(size: 11, weight: .semibold))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(PaperPalette.chip, in: RoundedRectangle(cornerRadius: 6))
            }
        }
        .frame(width: 24, height: 24)
        .help(app.name)
        .accessibilityLabel(app.name)
    }
}

// MARK: - AI

private struct AISection: View {
    let polish: PolishModel
    let screens: ScreenModels
    let cloud: CloudModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "AI")
            LanguageModelRows(cloud: cloud)
                .padding(.bottom, 6)
            SettingColumns {
                Text("Polish my words")
            } controls: {
                VStack(alignment: .leading, spacing: 4) {
                    Toggle(
                        "Polish my words",
                        isOn: Binding(get: { polish.isOn }, set: { polish.setOn($0) }))
                        .toggleStyle(.switch)
                        .labelsHidden()
                        .disabled(!polish.canToggle)
                        .accessibilityHint(polish.status)
                    Text(polish.status)
                        .font(Typography.caption)
                        .foregroundStyle(polish.isProblem ? Theme.alert : Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityHidden(true)
                }
            }
            .font(Typography.body)
            Text("Polish tidies a dictation's wording before it is typed: it keeps what you meant and never adds anything. It sends what you dictate to a language model, so it stays off until you turn it on and agree to where that is.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            SettingColumns {
                Text("Summaries and Ask")
            } controls: {
                VStack(alignment: .leading, spacing: 4) {
                    // Plain closures (the CI runner's Swift 6.3 crashes on some closure forms here).
                    Toggle(
                        "Summaries and Ask",
                        isOn: Binding(get: { screens.meetingsAIOn }, set: { screens.setMeetingsAI($0) }))
                        .toggleStyle(.switch)
                        .labelsHidden()
                        .disabled(!screens.canToggleMeetingsAI)
                        .accessibilityHint(screens.meetingsAIStatus)
                    Text(screens.meetingsAIStatus)
                        .font(Typography.caption)
                        .foregroundStyle(screens.meetingsConsent.isProblem ? Theme.alert : Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityHidden(true)
                }
            }
            .font(Typography.body)
            .padding(.top, 6)
            // Its own view, so its step and polish's never share one alert.
            .consentStep(screens.meetingsConsent, host: .settings)
            Text("A meeting's summary, what was promised in it, and Ask send its transcript to a language model, so they stay off until you turn them on and agree to where that is.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
        .polishConsent(polish, host: .settings)
    }
}

// MARK: - Meetings

private struct MeetingsSection: View {
    let permissions: PermissionsModel
    let meetings: MeetingModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Meetings")
            toggle(
                "Offer to record calls",
                detail: "When an app opens the microphone for a call, Inkwell asks whether to record it. It never records without you saying so.",
                isOn: meetings.detect, set: { meetings.setDetect($0) })
            if meetings.settingsFailed {
                Text("Couldn't read or save a meeting setting. It may not be what it shows.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
            }
            VStack(alignment: .leading, spacing: 8) {
                fact("Consent", "Tell the others in the call that you are recording. Inkwell shows while it records, and never hides that it does.")
                fact("You", "Your microphone, as \u{201C}Hear you\u{201D} allows.")
                fact("Them", "For a call you record when Inkwell offers, the call app's own sound. With Record now, or when Inkwell can't hear the call app alone, everything this Mac plays, and Inkwell says so. As \u{201C}Hear the others\u{201D} allows.")
                fact("Headphones", "With Bluetooth headphones, Automatic records the Mac's own microphone: a headset microphone carries only call-quality sound. Settings > Sound picks another.")
                fact("Where", "Recordings and transcripts stay on this Mac. Nothing is sent anywhere unless you add your own key for a model online.")
            }
            if permissions.state(.hearTheOthers).isAlert {
                Text(PermissionCard.hearTheOthers.offDetail)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
            }
        }
    }

    private func toggle(
        _ title: String, detail: String, isOn: Bool, set: @escaping @MainActor @Sendable (Bool) -> Void
    ) -> some View {
        SettingColumns {
            Text(title)
        } controls: {
            VStack(alignment: .leading, spacing: 4) {
                // A closure literal, not `set` itself: handing the main-actor closure straight to
                // Binding's generic setter makes Swift 6.3 (the CI runner's Xcode 26.6) crash
                // emitting the isolation thunk ("SmallVector unable to grow").
                Toggle(title, isOn: Binding(get: { isOn }, set: { set($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .accessibilityHint(detail)
                Text(detail)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityHidden(true)
            }
        }
        .font(Typography.body)
    }

    private func fact(_ label: String, _ text: String) -> some View {
        SettingColumns {
            Text(label).font(.system(.body, weight: .semibold))
        } controls: {
            Text(text).font(Typography.body).foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
    }
}

// MARK: - Models

private struct ModelsSection: View {
    let catalogue: CatalogueModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Models")
            if catalogue.failed {
                Text(CatalogueModel.failedText).foregroundStyle(Theme.alert)
            }
            ForEach(CatalogueModel.jobs, id: \.self) { job in
                let line = catalogue.line(job)
                SettingColumns {
                    Text(CatalogueModel.title(job))
                        .font(.system(.body, weight: .semibold))
                } controls: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(line.engineText)
                            .foregroundStyle(line.engine == nil ? Theme.secondaryText : Theme.text)
                        if let accuracy = line.accuracy {
                            Text(accuracy)
                                .font(Typography.caption)
                                .foregroundStyle(Theme.secondaryText)
                        }
                    }
                }
                .padding(.vertical, 6)
                .accessibilityElement(children: .combine)
            }
            if !catalogue.models.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Paper.Eyebrow(text: "Downloadable")
                    ForEach(catalogue.models, id: \.id) { model in
                        ModelDownloadRow(catalogue: catalogue, model: model)
                    }
                    Text("Nothing is downloaded until you press Download. Downloads run one at a time.")
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                }
                .padding(.top, 6)
            }
            // Where the accuracy comes from, only when a row shows one.
            if CatalogueModel.jobs.contains(where: { catalogue.line($0).accuracy != nil }) {
                Text("Accuracy is measured on public test sets: AMI meetings and FLEURS English.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
            }
        }
    }
}

/// A catalogue model: its name, licence, size and where it comes from, and its download: a
/// Download button while it is not on this Mac, its bar while it downloads,
/// and why it failed, with Retry. Settings > Models lists these (the first run's step shows choices: ModelChoices.swift).
struct ModelDownloadRow: View {
    let catalogue: CatalogueModel
    let model: CatalogueEntry

    var body: some View {
        let name = CatalogueModel.name(model.id)
        let state = catalogue.download(of: model)
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(name).font(.system(.body, weight: .semibold))
                Spacer(minLength: 8)
                trailing(state, name: name)
            }
            Text(facts)
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
            if case .failed(let why) = state {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text("Couldn't download it: \(why)")
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    Button("Retry") { catalogue.download([model.id]) }
                        .accessibilityLabel("Retry downloading \(name)")
                }
            }
        }
        .foregroundStyle(Theme.text)
        .padding(.vertical, 4)
    }

    /// Its licence, its size, and where it comes from.
    private var facts: String {
        var parts = [model.licence, Self.size(model.sizeBytes)]
        if let source = CatalogueModel.source(model.id) {
            parts.append("from \(source)")
        }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private func trailing(_ state: CatalogueModel.Download, name: String) -> some View {
        switch state {
        case .installed:
            Text("On this Mac").font(Typography.caption).foregroundStyle(Theme.secondaryText)
        case .notInstalled:
            Button("Download") { catalogue.download([model.id]) }
                .accessibilityLabel("Download \(name), \(facts)")
        case .waiting:
            Text("Waiting").font(Typography.caption).foregroundStyle(Theme.secondaryText)
        case .downloading(let progress?):
            HStack(spacing: 8) {
                ProgressView(value: progress.fraction).frame(width: 120)
                Text("\(Self.size(progress.done)) of \(Self.size(progress.total))")
                    .font(Typography.caption)
                    .monospacedDigit()
                    .foregroundStyle(Theme.secondaryText)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Downloading \(name)")
            .accessibilityValue("\(Int(progress.fraction * 100)) percent")
        case .downloading(nil):
            Text("Starting…").font(Typography.caption).foregroundStyle(Theme.secondaryText)
        case .failed:
            // Said on its own line, with Retry.
            EmptyView()
        }
    }

    static func size(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}

// MARK: - Storage

private struct StorageSection: View {
    let storage: StorageModel
    let meetings: MeetingModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Storage")
            SettingColumns {
                Text("Keep records")
            } controls: {
                VStack(alignment: .leading, spacing: 4) {
                    Picker("Keep records", selection: Binding(
                        get: { meetings.retention ?? .forever }, set: { meetings.setRetention($0) }
                    )) {
                        ForEach(Retention.allCases) { Text($0.title).tag($0) }
                    }
                    .labelsHidden()
                    .fixedSize()
                    .disabled(meetings.retention == nil)
                    Text("Older meetings and dictations are deleted with their recordings: their words are overwritten in the library's files, not only hidden. Anything you imported is kept. Nothing is deleted while it is forever.")
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .font(Typography.body)
            if let dir = storage.dataDirectory {
                HStack(spacing: 12) {
                    Text((dir.path as NSString).abbreviatingWithTildeInPath)
                        .font(Typography.timestamp)
                        .foregroundStyle(Theme.secondaryText)
                        .textSelection(.enabled)
                    Button("Show in Finder") { storage.showInFinder() }
                }
            }
            if let sizes = storage.sizes {
                row("Library", sizes.library)
                row("Recordings", sizes.recordings)
                row("Models", sizes.models)
            } else {
                ProgressView().controlSize(.small).accessibilityLabel("Measuring")
            }
        }
    }

    /// "0 KB" rather than the formatter's "Zero KB".
    static func size(_ bytes: Int64) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        formatter.allowsNonnumericFormatting = false
        return formatter.string(fromByteCount: bytes)
    }

    private func row(_ label: String, _ bytes: Int64) -> some View {
        SettingColumns {
            Text(label)
        } controls: {
            Text(Self.size(bytes))
                .foregroundStyle(Theme.secondaryText)
        }
        .font(Typography.body)
        .accessibilityElement(children: .combine)
    }
}

// MARK: - About

private struct AboutSection: View {
    @Environment(CoreStore.self) private var store

    private var version: String {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? "development build"
        return "Inkwell \(short)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionTitle(text: "About")
            Text(version).font(.system(.body, weight: .semibold))
            Text(CoreStatusText.line(for: store.status))
                .font(Typography.caption).foregroundStyle(Theme.secondaryText)
            Paper.Eyebrow(text: "Models").padding(.top, 8)
            ForEach(Notices.models) { model in
                NoticeRow(
                    title: "\(model.name), by \(model.author) (\(model.licence))",
                    detail: model.use, text: model.notice)
            }
            Paper.Eyebrow(text: "Code that ships inside Inkwell").padding(.top, 8)
            ForEach(Notices.components) { notice in
                NoticeRow(title: "\(notice.name) (\(notice.licence))", detail: notice.role, text: notice.text)
            }
            RustLibrariesRow()
        }
    }
}

/// The Rust crates linked into the core: one disclosure for all of them, and inside it a row per
/// crate that opens onto its licence text, like the rows above. The list scrolls in its own
/// bounded view, as each licence text does, so its rows are built lazily whatever holds the
/// section, and an open list does not stretch the Settings page by a hundred rows.
private struct RustLibrariesRow: View {
    var body: some View {
        DisclosureGroup {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    ForEach(RustNotices.crates) { notice in
                        NoticeRow(title: notice.title, detail: notice.detail, text: notice.text)
                    }
                }
                .padding(.vertical, 8)
            }
            .frame(maxHeight: 420)
        } label: {
            VStack(alignment: .leading, spacing: 1) {
                Text(RustNotices.heading).font(.system(.callout, weight: .medium)).foregroundStyle(Theme.text)
                Text("The open-source crates compiled into Inkwell's core, each with its licence.")
                    .font(Typography.caption).foregroundStyle(Theme.secondaryText)
            }
            .accessibilityElement(children: .combine)
        }
    }
}

private struct NoticeRow: View {
    let title: String
    let detail: String
    let text: String?

    var body: some View {
        if let text {
            DisclosureGroup {
                ScrollView {
                    Text(text)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(Theme.text)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 220)
            } label: {
                label
            }
        } else {
            label
        }
    }

    private var label: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title).font(.system(.callout, weight: .medium)).foregroundStyle(Theme.text)
            Text(detail).font(Typography.caption).foregroundStyle(Theme.secondaryText)
        }
        .accessibilityElement(children: .combine)
    }
}
