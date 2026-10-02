// Settings: General (open at login, updates, Inkwell 0.2's history), Appearance, permissions with
// their live state, dictation's keys, modes, snippets and voice commands (PhrasesSections), AI (the
// language model you bring, local-only mode, polish, summaries and Ask), meetings, models (with
// measured accuracy, and Download for those not on this Mac), storage, and About with every notice
// the app ships.
import AppleEngines
import InkBridge
import SwiftUI

enum SettingsSection: String, CaseIterable, Identifiable {
    case general
    case appearance
    case permissions
    case dictation
    case modes
    case snippets
    case voiceCommands
    case ai
    case meetings
    case models
    case storage
    case about

    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: "General"
        case .appearance: "Appearance"
        case .permissions: "Permissions"
        case .dictation: "Dictation"
        case .modes: "Modes"
        case .snippets: "Snippets"
        case .voiceCommands: "Voice commands"
        case .ai: "AI"
        case .meetings: "Meetings"
        case .models: "Models"
        case .storage: "Storage"
        case .about: "About"
        }
    }
}

struct SettingsScreen: View {
    @Environment(ScreenModels.self) private var screens
    @State private var section: SettingsSection? = .general
    /// The section a click scrolled to: it stays selected while any of it is in view, as the last
    /// sections cannot scroll to the top.
    @State private var clicked: SettingsSection?
    /// The selection the scrolling made, which must not scroll the page again.
    @State private var followed: SettingsSection?

    var body: some View {
        HStack(spacing: 0) {
            List(SettingsSection.allCases, selection: $section) { section in
                Text(section.title).tag(section)
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            .frame(width: 188)
            .accessibilityLabel("Settings sections")
            Rectangle().fill(PaperPalette.border).frame(width: 1).accessibilityHidden(true)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 30) {
                        GeneralSection(screens: screens).id(SettingsSection.general)
                        AppearanceSection(theme: screens.theme).id(SettingsSection.appearance)
                        PermissionsSection(permissions: screens.permissions).id(SettingsSection.permissions)
                        DictationSection(screens: screens, dictation: screens.dictation, permissions: screens.permissions)
                            .id(SettingsSection.dictation)
                        ModesSection(modes: screens.modes).id(SettingsSection.modes)
                        SnippetsSection(snippets: screens.snippets).id(SettingsSection.snippets)
                        VoiceCommandsSection(commands: screens.voiceCommands).id(SettingsSection.voiceCommands)
                        AISection(polish: screens.polish, screens: screens, cloud: screens.cloud).id(SettingsSection.ai)
                        MeetingsSection(permissions: screens.permissions, meetings: screens.meetings)
                            .id(SettingsSection.meetings)
                        ModelsSection(catalogue: screens.catalogue).id(SettingsSection.models)
                        StorageSection(storage: screens.storage, meetings: screens.meetings)
                            .id(SettingsSection.storage)
                        AboutSection().id(SettingsSection.about)
                    }
                    // The sections are the scroll's targets, for the list to follow (below).
                    .scrollTargetLayout()
                    .frame(maxWidth: 760, alignment: .leading)
                    .padding(.horizontal, 40)
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
        }
        .onDisappear { screens.permissions.screenDisappeared() }
    }

    /// Selects `next` for the scrolling, without scrolling.
    private func follow(_ next: SettingsSection?) {
        guard let next, next != section else { return }
        followed = next
        section = next
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
            PermissionCards(permissions: permissions)
        }
    }
}

/// The four cards, in one frame (Settings and onboarding).
struct PermissionCards: View {
    let permissions: PermissionsModel

    var body: some View {
        VStack(spacing: 0) {
            ForEach(PermissionCard.allCases) { card in
                PermissionRow(card: card, state: permissions.state(card)) { permissions.request(card) }
                if card != PermissionCard.allCases.last {
                    Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: Glow.Radius.card, style: .continuous))
        .paperCard()
    }
}

private struct PermissionRow: View {
    let card: PermissionCard
    let state: CardState
    let request: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            icon.frame(width: 22, height: 22).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(card.title).font(.system(.body, weight: .semibold)).foregroundStyle(Theme.text)
                Text(state.isAlert ? card.offDetail : card.detail)
                    .font(Typography.caption)
                    .foregroundStyle(state.isAlert ? Theme.text : Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            trailing
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 13)
        .background(state.isAlert ? PaperPalette.alertCard : Color.clear)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(card.title): \(spoken)")
        .accessibilityAction(named: actionTitle ?? "Check") { request() }
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

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Dictation")
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Dictation").frame(width: 150, alignment: .leading)
                Toggle("Dictation", isOn: Binding(get: { dictation.isOn }, set: { dictation.setOn($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                Text(dictation.isOn ? "The keys below are Inkwell's" : "Off: the keys do what they did before")
                    .foregroundStyle(Theme.secondaryText)
            }
            .font(Typography.body)
            .padding(.vertical, 5)
            .accessibilityElement(children: .contain)
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Dictate").frame(width: 150, alignment: .leading)
                Picker("Dictate", selection: Binding(get: { dictation.key }, set: { dictation.setKey($0) })) {
                    ForEach(DictationModel.keys) { key in
                        Text(key.name).tag(key.token)
                    }
                }
                .labelsHidden()
                .fixedSize()
                Key(text: DictationModel.key(dictation.key)?.cap ?? dictation.key)
                Text("hold, speak, let go").foregroundStyle(Theme.secondaryText)
            }
            .font(Typography.body)
            .padding(.vertical, 5)
            .accessibilityElement(children: .contain)
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Edit a selection").frame(width: 150, alignment: .leading)
                Picker("Edit a selection", selection: Binding(
                    get: { dictation.editKey ?? "off" },
                    set: { screens.chooseEditKey($0 == "off" ? nil : $0) }
                )) {
                    Text("Off").tag("off")
                    ForEach(DictationModel.keys.filter { $0.token != dictation.key }) { key in
                        Text(key.name).tag(key.token)
                    }
                }
                .labelsHidden()
                .fixedSize()
                if let edit = dictation.editKey, dictation.editKeyProblem == nil {
                    Key(text: DictationModel.key(edit)?.cap ?? edit)
                }
                Text("select text, hold, say what to change").foregroundStyle(Theme.secondaryText)
            }
            .font(Typography.body)
            .padding(.vertical, 5)
            .accessibilityElement(children: .contain)
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
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text(row.isDefault && modes.rows.count > 1 ? "Everywhere else" : row.name)
                        .font(.system(.body, weight: .semibold))
                        .frame(width: 150, alignment: .leading)
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
                    Spacer(minLength: 0)
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
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Polish my words").frame(width: 150, alignment: .leading)
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
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Summaries and Ask").frame(width: 150, alignment: .leading)
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
            toggle(
                "Use the headset's microphone",
                detail: "With Bluetooth headphones, record their own microphone instead of the Mac's. It carries only call-quality sound.",
                isOn: meetings.headsetMic, set: { meetings.setHeadsetMic($0) })
            if meetings.settingsFailed {
                Text("Couldn't read or save a meeting setting. It may not be what it shows.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
            }
            VStack(alignment: .leading, spacing: 8) {
                fact("Consent", "Tell the others in the call that you are recording. Inkwell shows while it records, and never hides that it does.")
                fact("You", "Your microphone, as \u{201C}Hear you\u{201D} allows.")
                fact("Them", "For a call you record when Inkwell offers, the call app's own sound. With Record now, or when Inkwell can't hear the call app alone, everything this Mac plays, and Inkwell says so. As \u{201C}Hear the others\u{201D} allows.")
                fact("Headphones", "With Bluetooth headphones, Inkwell records the Mac's own microphone: a headset microphone carries only call-quality sound.")
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
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(title).frame(width: 150, alignment: .leading)
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
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(label).font(.system(.body, weight: .semibold)).frame(width: 150, alignment: .leading)
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
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text(CatalogueModel.title(job))
                        .font(.system(.body, weight: .semibold))
                        .frame(width: 150, alignment: .leading)
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
                        ModelDownloadRow(catalogue: catalogue, model: model, offersDownload: true)
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
/// Download button (when `offersDownload`) while it is not on this Mac, its bar while it downloads,
/// and why it failed, with Retry. Settings > Models and the first run's Models step list these.
struct ModelDownloadRow: View {
    let catalogue: CatalogueModel
    let model: CatalogueEntry
    let offersDownload: Bool

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
            if offersDownload {
                Button("Download") { catalogue.download([model.id]) }
                    .accessibilityLabel("Download \(name), \(facts)")
            }
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
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Keep records").frame(width: 150, alignment: .leading)
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
        HStack(spacing: 12) {
            Text(label).frame(width: 150, alignment: .leading)
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
