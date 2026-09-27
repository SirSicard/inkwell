// Settings: permissions with their live state, the voice key, modes, AI (polish), meetings,
// models (read-only, with measured accuracy), storage, and About with every notice the app ships.
import AppleEngines
import InkBridge
import SwiftUI

enum SettingsSection: String, CaseIterable, Identifiable {
    case permissions
    case voice
    case modes
    case ai
    case meetings
    case models
    case storage
    case about

    var id: String { rawValue }

    var title: String {
        switch self {
        case .permissions: "Permissions"
        case .voice: "Voice"
        case .modes: "Modes"
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
    @State private var section: SettingsSection? = .permissions

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
                        PermissionsSection(permissions: screens.permissions).id(SettingsSection.permissions)
                        VoiceSection().id(SettingsSection.voice)
                        ModesSection(modes: screens.modes).id(SettingsSection.modes)
                        AISection(polish: screens.polish).id(SettingsSection.ai)
                        MeetingsSection(permissions: screens.permissions).id(SettingsSection.meetings)
                        ModelsSection(catalogue: screens.catalogue).id(SettingsSection.models)
                        StorageSection(storage: screens.storage).id(SettingsSection.storage)
                        AboutSection().id(SettingsSection.about)
                    }
                    .frame(maxWidth: 760, alignment: .leading)
                    .padding(.horizontal, 40)
                    .padding(.vertical, 28)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .onChange(of: section) { _, section in
                    if let section { proxy.scrollTo(section, anchor: .top) }
                }
            }
        }
        .onAppear {
            screens.permissions.screenAppeared()
            screens.modes.load()
            screens.polish.load()
            screens.catalogue.requery()
            screens.storage.measure()
        }
        .onDisappear { screens.permissions.screenDisappeared() }
    }
}

/// A section's heading.
private struct SectionTitle: View {
    let text: String
    var note: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(text)
                .font(.system(.title2, weight: .semibold))
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
        .clipShape(RoundedRectangle(cornerRadius: 14))
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
            Text(state == .allowed ? "Allowed" : "Checking…")
                .font(Typography.timestamp)
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

// MARK: - Voice

private struct VoiceSection: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Voice")
            HStack(spacing: 12) {
                Text("Dictate").frame(width: 150, alignment: .leading)
                Key(text: "fn")
                Text("hold, speak, let go").foregroundStyle(Theme.secondaryText)
            }
            .font(Typography.body)
            .padding(.vertical, 9)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Dictate: hold the fn key, speak, and let go")
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
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text(row.isDefault && modes.rows.count > 1 ? "Everywhere else" : row.name)
                        .font(.system(.body, weight: .semibold))
                        .frame(width: 150, alignment: .leading)
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 6) {
                            ForEach(row.traits, id: \.self) { Paper.Chip(text: $0) }
                        }
                        if row.apps.isEmpty {
                            Text(row.isDefault ? "Every app no other mode names" : "No apps")
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

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "AI")
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
                        .foregroundStyle(polish.keepsTimingOut ? Theme.alert : Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityHidden(true)
                }
            }
            .font(Typography.body)
            Text("Polish tidies a dictation's wording before it is typed: it keeps what you meant and never adds anything.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
    }
}

// MARK: - Meetings

private struct MeetingsSection: View {
    let permissions: PermissionsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Meetings")
            VStack(alignment: .leading, spacing: 8) {
                fact("You", "Your microphone, as \u{201C}Hear you\u{201D} allows.")
                fact("Them", "The sound of the call from this Mac, as \u{201C}Hear the others\u{201D} allows.")
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
            SectionTitle(text: "Models", note: "Read-only")
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
                        Text(line.engine ?? (line.known ? "Nothing installed yet" : "Checking…"))
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
                        Text("\(CatalogueModel.name(model.id)) · \(model.licence) · \(ByteCountFormatter.string(fromByteCount: model.sizeBytes, countStyle: .file)) · \(model.installed ? "installed" : "not installed")")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                    }
                }
                .padding(.top, 6)
            }
            Text("Accuracy is measured on public test sets: AMI meetings and FLEURS English.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
    }
}

// MARK: - Storage

private struct StorageSection: View {
    let storage: StorageModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Storage")
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
    @Environment(Updates.self) private var updates

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
            updatesRow
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

    @ViewBuilder private var updatesRow: some View {
        switch updates.availability {
        case .on:
            HStack(spacing: 12) {
                Toggle("Check for updates automatically", isOn: Binding(
                    get: { updates.checksAutomatically }, set: { updates.checksAutomatically = $0 }))
                Button("Check Now") { updates.checkForUpdates() }.disabled(!updates.canCheck)
            }
        case .off(let reason):
            Text(reason.explanation).font(Typography.caption).foregroundStyle(Theme.secondaryText)
        }
    }
}

/// The Rust crates linked into the core: one disclosure for all of them, and inside it a row per
/// crate that opens onto its licence text, like the rows above. Lazy, so a closed list, or the
/// part scrolled past, builds no rows.
private struct RustLibrariesRow: View {
    var body: some View {
        DisclosureGroup {
            LazyVStack(alignment: .leading, spacing: 12) {
                ForEach(RustNotices.crates) { notice in
                    NoticeRow(title: notice.title, detail: notice.detail, text: notice.text)
                }
            }
            .padding(.top, 8)
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
