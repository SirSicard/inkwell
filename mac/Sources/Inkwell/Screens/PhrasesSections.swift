// Settings > Snippets and Settings > Voice commands (PhrasesModel), and the one-time note about
// Inkwell 0.2's dictation key in Settings > Dictation. The user's own words are shown as plain text:
// nothing here is a link.
import InkBridge
import SwiftUI

// MARK: - Snippets

struct SnippetsSection: View {
    let snippets: SnippetsModel

    @State private var trigger = ""
    @State private var expansion = ""
    @State private var category = ""
    /// The snippet being edited, as edited so far.
    @State private var editing: SnippetDraft?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Snippets", note: "Say a trigger, get the text")
            if let failure = snippets.failure {
                FailureLine(text: failure, canStartOver: snippets.unreadable, startOver: { snippets.startOver() })
            }
            if snippets.fromImport {
                Text("Brought over from Inkwell 0.2.").font(Typography.caption).foregroundStyle(Theme.secondaryText)
            }
            if snippets.loaded && snippets.rows.isEmpty {
                Text("No snippets yet.").font(Typography.caption).foregroundStyle(Theme.secondaryText)
            }
            ForEach(snippets.rows) { row in
                if editing?.id == row.id {
                    editor
                } else {
                    SnippetRow(
                        row: row,
                        setEnabled: { snippets.setEnabled(row.id, $0) },
                        edit: { editing = row },
                        delete: { snippets.delete(row.id) })
                        .disabled(!snippets.loaded)
                }
                Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
            }
            addForm
            Text("A trigger is matched as whole words, in any case, after the dictionary. {date} and {time} in the text are filled in when it goes in.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var editor: some View {
        if let draft = editing {
            VStack(alignment: .leading, spacing: 6) {
                TextField("Trigger", text: Binding(get: { editing?.trigger ?? "" }, set: { editing?.trigger = $0 }))
                TextField("Text it becomes", text: Binding(get: { editing?.expansion ?? "" }, set: { editing?.expansion = $0 }), axis: .vertical)
                    .lineLimit(1...6)
                TextField("Category (optional)", text: Binding(get: { editing?.category ?? "" }, set: { editing?.category = $0 }))
                HStack {
                    Button("Save") {
                        snippets.update(draft)
                        editing = nil
                    }
                    .disabled(draft.trigger.trimmingCharacters(in: .whitespaces).isEmpty)
                    .keyboardShortcut(.defaultAction)
                    Button("Cancel") { editing = nil }
                }
            }
            .textFieldStyle(.roundedBorder)
            .font(Typography.body)
            .padding(.vertical, 8)
        }
    }

    private var addForm: some View {
        LineOrStack(minWidth: 480) {
            TextField("Trigger", text: $trigger).frame(width: 150)
            TextField("Text it becomes", text: $expansion, axis: .vertical).lineLimit(1...4)
            TextField("Category", text: $category).frame(width: 110)
            Button("Add") {
                snippets.add(trigger: trigger, expansion: expansion, category: category)
                trigger = ""
                expansion = ""
                category = ""
            }
            .disabled(trigger.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .textFieldStyle(.roundedBorder)
        .font(Typography.body)
        .padding(.top, 4)
        // Nothing is added to a list that has not been read (SnippetsModel refuses too).
        .disabled(!snippets.loaded)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Add a snippet")
    }
}

private struct SnippetRow: View {
    let row: SnippetDraft
    let setEnabled: @MainActor (Bool) -> Void
    let edit: @MainActor () -> Void
    let delete: @MainActor () -> Void

    var body: some View {
        SettingColumns {
            Text(verbatim: row.trigger)
                .font(.system(.body, design: .monospaced, weight: .semibold))
                .foregroundStyle(row.enabled ? Theme.text : Theme.secondaryText)
        } controls: {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: row.expansion)
                        .foregroundStyle(row.enabled ? Theme.text : Theme.secondaryText)
                        .lineLimit(3)
                    if !row.category.isEmpty {
                        Paper.Chip(text: row.category)
                    }
                }
                Spacer(minLength: 0)
                // Plain closures around the model's calls: Swift 6.3 (the CI runner's) crashes on a
                // main-actor function handed to Binding's setter.
                Toggle("On", isOn: Binding(get: { row.enabled }, set: { setEnabled($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .controlSize(.small)
                Button("Edit") { edit() }
                Button("Delete", role: .destructive) { delete() }
            }
        }
        .font(Typography.body)
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Snippet \(row.trigger)\(row.enabled ? "" : ", off"): \(row.expansion)\(row.category.isEmpty ? "" : ", \(row.category)")")
    }
}

// MARK: - Voice commands

struct VoiceCommandsSection: View {
    let commands: VoiceCommandsModel

    @State private var wake = ""
    @State private var triggers = ""
    @State private var action: CommandAction = .insertText
    @State private var value = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Voice commands")
            if let failure = commands.failure {
                FailureLine(text: failure, canStartOver: commands.unreadable, startOver: { commands.startOver() })
            }
            SettingColumns {
                Text("Voice commands")
            } controls: {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Toggle("Voice commands", isOn: Binding(get: { commands.enabled }, set: { commands.setEnabled($0) }))
                        .toggleStyle(.switch)
                        .labelsHidden()
                        .disabled(!commands.loaded)
                    Text(commands.enabled
                        ? "Say \u{201C}\(commands.wakePrefix)\u{201D}, then a command"
                        : "Off: everything you say is dictated")
                        .foregroundStyle(Theme.secondaryText)
                }
            }
            .font(Typography.body)
            .accessibilityElement(children: .contain)
            SettingColumns {
                Text("Wake word")
            } controls: {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    TextField("Wake word", text: $wake)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 160)
                        .onSubmit { commands.setWakePrefix(wake) }
                        .disabled(!commands.loaded)
                    Button("Save") { commands.setWakePrefix(wake) }
                        .disabled(!commands.loaded || wake.trimmingCharacters(in: .whitespaces).isEmpty
                            || wake.trimmingCharacters(in: .whitespaces).lowercased() == commands.wakePrefix)
                }
            }
            .font(Typography.body)
            .onAppear { wake = commands.wakePrefix }
            .onChange(of: commands.wakePrefix) { _, now in wake = now }
            if commands.fromImport {
                Text("Brought over from Inkwell 0.2.").font(Typography.caption).foregroundStyle(Theme.secondaryText)
            }
            ForEach(commands.rows) { row in
                CommandRow(
                    row: row,
                    setEnabled: { commands.setCommandEnabled(row.id, $0) },
                    delete: { commands.delete(row.id) })
                    .disabled(!commands.loaded)
                Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
            }
            addForm
        }
    }

    private var addForm: some View {
        LineOrStack(minWidth: 520) {
            TextField("Phrases, comma-separated", text: $triggers).frame(width: 200)
            Picker("Does", selection: $action) {
                Text("Type text").tag(CommandAction.insertText)
                Text("Switch style").tag(CommandAction.changeStyle)
            }
            .labelsHidden()
            .fixedSize()
            TextField(action == .insertText ? "Text to type" : "Style or mode name", text: $value)
            Button("Add") {
                commands.add(triggers: triggers, action: action, value: value)
                triggers = ""
                value = ""
            }
            .disabled(VoiceCommandsModel.phrases(triggers).isEmpty || value.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .textFieldStyle(.roundedBorder)
        .font(Typography.body)
        .padding(.top, 4)
        .disabled(!commands.loaded)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Add a voice command")
    }
}

private struct CommandRow: View {
    let row: VoiceCommandDraft
    let setEnabled: @MainActor (Bool) -> Void
    let delete: @MainActor () -> Void

    var body: some View {
        SettingColumns(titleWidth: 220) {
            Text(verbatim: row.triggers.joined(separator: " \u{00B7} "))
                .font(.system(.body, design: .monospaced))
                .foregroundStyle(row.enabled ? Theme.text : Theme.secondaryText)
        } controls: {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: VoiceCommandsModel.describe(row))
                        .foregroundStyle(row.enabled ? Theme.text : Theme.secondaryText)
                        .lineLimit(2)
                    if !row.carriedOut {
                        Text("Its action comes in a later version. While on, saying it types nothing.")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                    }
                }
                Spacer(minLength: 0)
                Toggle("On", isOn: Binding(get: { row.enabled }, set: { setEnabled($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .controlSize(.small)
                Button("Delete", role: .destructive) { delete() }
            }
        }
        .font(Typography.body)
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(
            "Voice command \(row.triggers.joined(separator: ", ")): \(VoiceCommandsModel.describe(row))"
                + (row.enabled ? "" : ", off")
                + (row.carriedOut ? "" : ". Its action comes in a later version; while on, saying it types nothing"))
    }
}

/// A list's failure, and, when the stored list cannot be read, the one way to replace it.
private struct FailureLine: View {
    let text: String
    let canStartOver: Bool
    let startOver: @MainActor () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(text).font(Typography.caption).foregroundStyle(Theme.alert)
            if canStartOver {
                Button("Start over (replaces the damaged list)") { startOver() }
                    .help("The stored list can\u{2019}t be read. This replaces it with an empty one.")
            }
        }
    }
}

// MARK: - The 0.2 key note (Settings > Dictation)

struct ImportKeyNoteView: View {
    let model: ImportNoteModel
    /// The dictation key in use now, by name.
    let currentKey: String

    var body: some View {
        if let note = model.note {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Image(systemName: "info.circle").foregroundStyle(Theme.secondaryText).accessibilityHidden(true)
                Text(ImportNoteModel.text(note, currentKey: currentKey))
                    .font(Typography.caption)
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                Button("Got it") { model.dismiss() }
            }
            .padding(12)
            .paperCard()
            .accessibilityElement(children: .contain)
        }
    }
}
