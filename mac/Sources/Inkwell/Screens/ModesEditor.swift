// Settings > Modes: the rows (name, chips, apps, Edit… and Delete), "Add a mode", and the editor,
// a sheet: the name, how it writes, its switches, its polish instructions, its model and the apps
// it is used in. The editor's model is ModesModel; nothing is saved until Save, and Cancel or
// Escape leaves the mode as it was. Polish's consent for a mode's own model is asked in the sheet,
// before the save that needs it.
import AppKit
import InkBridge
import SwiftUI
import UniformTypeIdentifiers

struct ModesSection: View {
    let modes: ModesModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Modes", note: "Picked by the app you're typing in")
            if modes.failed {
                Text(ModesModel.loadFailedText).font(Typography.caption).foregroundStyle(Theme.alert)
            }
            if let problem = modes.problem {
                Text(problem)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if modes.unreadable {
                Button("Start over (replaces the damaged modes)") { modes.startOver() }
                    .help("Your stored modes can\u{2019}t be read. This replaces them with the default mode.")
            }
            ForEach(modes.rows) { row in
                ModeRowView(modes: modes, row: row)
                Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
            }
            Button("Add a mode") { modes.add() }
                .disabled(modes.failed)
                .accessibilityHint("Opens the mode editor")
        }
        .sheet(item: Binding(get: { modes.editor }, set: { if $0 == nil { modes.closeEditor() } })) { editor in
            ModeEditorSheet(modes: modes, editor: editor).followsAppMode()
        }
        // The row is handed to the buttons (presenting:): the delete is of the mode the question named.
        .confirmationDialog(
            modes.deleting.map { "Delete \u{201C}\($0.name)\u{201D}?" } ?? "",
            isPresented: Binding(get: { modes.deleting != nil }, set: { if !$0 { modes.deleting = nil } }),
            titleVisibility: .visible,
            presenting: modes.deleting
        ) { _ in
            Button("Delete", role: .destructive) { modes.delete() }
            Button("Cancel", role: .cancel) { modes.deleting = nil }
                .keyboardShortcut(.defaultAction)
        } message: { row in
            Text(ModesModel.deleteMessage(row))
        }
        .alert(
            modes.confirming.map(ModesModel.confirmTitle) ?? "",
            isPresented: Binding(get: { modes.confirming != nil }, set: { if !$0 { modes.confirming = nil } }),
            presenting: modes.confirming
        ) { confirming in
            Button("Cancel", role: .cancel) { modes.confirming = nil }
            Button(ModesModel.confirmButton(confirming)) { modes.confirmAllow() }
                .keyboardShortcut(.defaultAction)
        } message: { confirming in
            Text(modes.confirmMessage(confirming))
        }
    }
}

/// One mode: its name | its chips, its apps, what stops its polish, and its buttons, each on a line
/// of its own (beside the name there is little room at 720 pt).
private struct ModeRowView: View {
    let modes: ModesModel
    let row: ModeRow

    var body: some View {
        SettingColumns {
            Text(row.title).font(.system(.body, weight: .semibold))
        } controls: {
            VStack(alignment: .leading, spacing: 8) {
                FlowRow(spacing: 6) {
                    ForEach(row.traits, id: \.self) { Paper.Chip(text: $0) }
                    if let chip = row.polish.chip {
                        Paper.Chip(text: chip, tone: row.polish.chipDimmed ? .muted : .neutral)
                            .help(row.polish.why ?? "")
                            .accessibilityLabel(row.polish.spokenChip ?? chip)
                    }
                }
                .accessibilityElement(children: .combine)
                apps
                if let note = row.polish.rowNote {
                    Text(note.text)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                        .fixedSize(horizontal: false, vertical: true)
                }
                FlowRow(spacing: 8) {
                    Button("Edit\u{2026}") { modes.edit(row.id) }
                        .accessibilityLabel("Edit \(row.title)")
                    if !row.isDefault {
                        Button("Delete") { modes.askDelete(row.id) }
                            .accessibilityLabel("Delete \(row.title)")
                    }
                    switch row.polish.rowNote?.fix {
                    case .confirm:
                        Button("Confirm\u{2026}") { modes.askConfirm(row.id) }
                            .accessibilityLabel("Confirm where \(row.title)'s model sends")
                    case .allow:
                        Button("Allow\u{2026}") { modes.askConfirm(row.id) }
                            .accessibilityLabel("Allow polish for \(row.title)")
                    case nil:
                        EmptyView()
                    }
                }
            }
        }
        .padding(.vertical, 10)
        // Contain, not combine: the buttons stay buttons VoiceOver can reach.
        .accessibilityElement(children: .contain)
        .accessibilityLabel(row.title)
    }

    @ViewBuilder private var apps: some View {
        if row.apps.isEmpty {
            Text(row.isDefault ? "Every app without a mode of its own" : "No apps: used only when you switch to it by voice.")
                .font(Typography.caption).foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            VStack(alignment: .leading, spacing: 4) {
                FlowRow(spacing: 6) {
                    ForEach(row.apps) { AppIcon(app: $0) }
                }
                .accessibilityHidden(true)
                Text(row.apps.map(\.name).joined(separator: ", "))
                    .font(Typography.caption).foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel("Used in \(row.apps.map(\.name).joined(separator: ", "))")
            }
        }
    }
}

/// An app's icon, or its first letter when it is not on this Mac.
struct AppIcon: View {
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

// MARK: - The editor

struct ModeEditorSheet: View {
    let modes: ModesModel
    @Bindable var editor: ModeEditor

    /// The sheet's size: as tall as the first-run sheet, narrower than the window at its 720-pt
    /// minimum. What does not fit scrolls; the buttons stay.
    static let size = CGSize(width: 540, height: 560)
    static let padding: CGFloat = 24
    /// The rows' name column.
    static let titleWidth: CGFloat = 130

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(editor.adding ? "Add a mode" : "Edit \u{201C}\(editor.original?.name ?? editor.name)\u{201D}")
                .font(Typography.heading)
                .foregroundStyle(Theme.text)
                .accessibilityAddTraits(.isHeader)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    nameRow
                    writesRow
                    switchesRow
                    modelRow
                    promptRow
                    appsRow
                }
                // Room for the first field's border and focus ring under the scroll's top edge.
                .padding(.top, 4)
                .padding(.trailing, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let error = editor.error {
                Text(error)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Cancel") { modes.closeEditor() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") { modes.save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(editor.saving)
                    .accessibilityLabel(editor.adding ? "Save the new mode" : "Save \(editor.name)")
            }
        }
        .font(Typography.body)
        .padding(Self.padding)
        .frame(width: Self.size.width, height: Self.size.height)
        .background(Theme.surface)
        .onChange(of: editor.error) { _, error in
            // Said as it appears: the line is under the fields, out of VoiceOver's way.
            if let error { AccessibilityNotification.Announcement(error).post() }
        }
        .alert(
            modes.consentTitle(editor),
            isPresented: Binding(get: { editor.consentStep != nil }, set: { if !$0 { modes.cancelConsentStep() } }),
            presenting: editor.consentStep
        ) { destination in
            Button("Cancel", role: .cancel) { modes.cancelConsentStep() }
                .accessibilityLabel("Cancel, and save nothing")
            Button(ConsentModel.button(.polish, destination)) { modes.allowAndSave() }
                .keyboardShortcut(.defaultAction)
        } message: { destination in
            Text(modes.consentMessage(destination))
        }
    }

    private func row<Controls: View>(_ title: String, @ViewBuilder controls: () -> Controls) -> some View {
        SettingColumns(titleWidth: Self.titleWidth) {
            Text(title)
        } controls: {
            VStack(alignment: .leading, spacing: 6) { controls() }
        }
        .accessibilityElement(children: .contain)
    }

    private func caption(_ text: String, problem: Bool = false) -> some View {
        Text(text)
            .font(Typography.caption)
            .foregroundStyle(problem ? Theme.alert : Theme.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var nameRow: some View {
        row("Name") {
            TextField("Name", text: $editor.name, prompt: Text("Chat, Email, Notes…"))
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("Name")
            caption("Say its name in a voice command to switch to it.")
            if editor.renamed {
                caption("Voice commands use the new name.")
            }
        }
    }

    private var writesRow: some View {
        row("Writes") {
            SegmentsOrMenu {
                Picker("Writes", selection: $editor.style) {
                    ForEach([ModeStyle.formal, .casual, .relaxed], id: \.self) { Text(ModesModel.style($0)).tag($0) }
                    // A style this build does not know stays until another is picked.
                    if editor.original?.style == .other {
                        Text(ModesModel.style(.other)).tag(ModeStyle.other)
                    }
                }
                .labelsHidden()
            }
        }
    }

    private var switchesRow: some View {
        Group {
            row("Clean up speech") {
                Toggle("Clean up speech", isOn: $editor.removeFillers)
                    .toggleStyle(.switch)
                    .labelsHidden()
                caption("Takes out fillers and stutters.")
            }
            row("Polish") {
                Toggle("Polish", isOn: $editor.polish)
                    .toggleStyle(.switch)
                    .labelsHidden()
                caption("Tidies the wording with a language model before it is typed.")
                if let note = modes.polishNote(editor) {
                    caption(note, problem: true)
                }
            }
        }
    }

    private var modelRow: some View {
        row("Polish with") {
            Picker("Polish with", selection: Binding(get: { editor.polishModel }, set: { picked in
                editor.polishModel = picked
                editor.confirmed = false
            })) {
                ForEach(modes.modelOptions(editor), id: \.id) { option in
                    Text(option.label).tag(option.id)
                }
            }
            .labelsHidden()
            // As wide as the row, no wider: a model's long name is cut in the button, whole in the menu.
            .frame(maxWidth: .infinity, alignment: .leading)
            if editor.polishModel?.hasPrefix("provider:") == true {
                TextField("Model", text: $editor.polishModelName,
                          prompt: Text(modes.providerModel(editor) ?? "The model chosen in AI"))
                    .labelsHidden()
                    .textFieldStyle(.roundedBorder)
                    .accessibilityLabel("Model at the provider")
                caption("Blank uses the model chosen in AI.")
            }
            let note = modes.modelNote(editor)
            caption(note.text, problem: note.isProblem)
            if modes.canConfirmInEditor(editor) {
                Button("Confirm") { modes.confirmInEditor(editor) }
                    .accessibilityLabel("Confirm where this mode's model sends")
            } else if editor.confirmed {
                caption("Confirmed. Saving records where it sends now.")
            }
        }
    }

    private var promptRow: some View {
        row("Polish instructions") {
            ZStack(alignment: .topLeading) {
                TextEditor(text: $editor.prompt)
                    .font(Typography.body)
                    .scrollContentBackground(.hidden)
                    .accessibilityLabel("Polish instructions")
                    .accessibilityHint("Blank uses the default.")
                if editor.prompt.isEmpty {
                    // The default, as the placeholder: what a blank one uses.
                    Text(modes.defaultPrompt)
                        .font(Typography.body)
                        .foregroundStyle(Theme.secondaryText)
                        .padding(.horizontal, 5)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .frame(height: 96)
            .clipped()
            .padding(4)
            .background(PaperPalette.card, in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(PaperPalette.border))
            HStack(alignment: .firstTextBaseline) {
                Button("Use the default") { editor.prompt = "" }
                    .disabled(editor.prompt.isEmpty)
                Spacer(minLength: 8)
                Text(editor.promptCount)
                    .font(Typography.caption)
                    .monospacedDigit()
                    .foregroundStyle(editor.promptTooLong ? Theme.alert : Theme.secondaryText)
                    .accessibilityLabel("\(editor.prompt.count) of \(ModesModel.promptLimit) characters")
            }
            caption("Blank uses the default.")
        }
    }

    private var appsRow: some View {
        row("Used in") {
            if editor.isDefault {
                caption("Every app without a mode of its own")
            } else {
                if editor.apps.isEmpty {
                    caption("No apps: used only when you switch to it by voice.")
                }
                ForEach(editor.apps, id: \.self) { identity in
                    let app = modes.label(identity)
                    HStack(spacing: 8) {
                        AppIcon(app: app).accessibilityHidden(true)
                        Text(app.name)
                        Spacer(minLength: 8)
                        Button {
                            modes.removeApp(identity, from: editor)
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.borderless)
                        .help("Remove \(app.name)")
                        .accessibilityLabel("Remove \(app.name)")
                    }
                    .accessibilityElement(children: .contain)
                }
                ForEach(modes.movingNotes(editor), id: \.self) { caption($0) }
                AddAppMenu(modes: modes, editor: editor)
            }
        }
    }
}

/// "Add an app": the apps running now (each in another mode says which), and Choose an App…, an
/// open panel on /Applications.
private struct AddAppMenu: View {
    let modes: ModesModel
    let editor: ModeEditor
    /// Read when the sheet shows and when an app starts or quits: no polling.
    @State private var offers: [(app: PickedApp, owner: String?)] = []

    var body: some View {
        Menu("Add an app") {
            Section("Running now") {
                if offers.isEmpty {
                    Text("No other apps are running")
                }
                ForEach(offers, id: \.app.id) { offer in
                    Button(offer.owner.map { "\(offer.app.name) \u{2014} in \($0)" } ?? offer.app.name) {
                        modes.addApp(offer.app.identity, to: editor)
                        refresh()
                    }
                }
            }
            Divider()
            Button("Choose an App\u{2026}") { choose() }
        }
        .fixedSize()
        .accessibilityLabel("Add an app")
        .onAppear(perform: refresh)
        .onReceive(NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.didLaunchApplicationNotification)) { _ in refresh() }
        .onReceive(NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.didTerminateApplicationNotification)) { _ in refresh() }
    }

    private func refresh() {
        offers = modes.runningOffers(editor)
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.directoryURL = URL(fileURLWithPath: "/Applications", isDirectory: true)
        panel.allowedContentTypes = [.application]
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Add"
        panel.message = "Choose an app for this mode"
        let picked: @MainActor (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else { return }
            modes.addChosen(url, to: editor)
            refresh()
        }
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window) { response in MainActor.assumeIsolated { picked(response) } }
        } else {
            panel.begin { response in MainActor.assumeIsolated { picked(response) } }
        }
    }
}
