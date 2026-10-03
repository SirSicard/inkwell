// Settings > General, Settings > Appearance, and the language model you bring (Settings > AI's
// first rows). Each control shows what its model holds; a change is sent, and the core's answer
// (or Sparkle's, or the login item's) is what shows.
import AppKit
import InkBridge
import SwiftUI

// MARK: - General

/// Open at Login, the menu bar, updates, and Inkwell 0.2's history.
struct GeneralSection: View {
    let screens: ScreenModels
    @Environment(Updates.self) private var updates
    /// Read when the section shows and when the app comes back to the front: the user may have
    /// approved it in System Settings meanwhile.
    @State private var login = LoginItem.State.off

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "General")
            SettingRow(title: "Open at Login") {
                Toggle("Open at Login", isOn: Binding(
                    get: { login == .on || login == .needsApproval },
                    set: { _ in
                        LoginItem.toggle()
                        login = LoginItem.state
                    }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .disabled(login == .unavailable)
                Text(loginDetail)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            SettingRow(title: "Menu bar") {
                Text("Inkwell always shows in the menu bar: dictation, meetings and the Drop work from there with this window closed, and its menu records, switches dictation and opens Settings.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            SettingRow(title: "Updates") {
                switch updates.availability {
                case .on:
                    Toggle("Check for updates automatically", isOn: Binding(
                        get: { updates.checksAutomatically }, set: { updates.checksAutomatically = $0 }))
                    Toggle("Download updates automatically", isOn: Binding(
                        get: { updates.downloadsAutomatically }, set: { updates.downloadsAutomatically = $0 }))
                        .disabled(!updates.checksAutomatically)
                    Button("Check Now") { updates.checkForUpdates() }
                        .disabled(!updates.canCheck)
                case .off(let reason):
                    Text(reason.explanation)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                }
            }
            // Inkwell 0.2's history, while there is some to import (or the look for it failed).
            if screens.import02.offered || screens.import02.checkFailed {
                Import02Card(model: screens.import02)
                    .padding(14)
                    .paperCard()
            }
        }
        .onAppear { login = LoginItem.state }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            login = LoginItem.state
        }
    }

    private var loginDetail: String {
        switch login {
        case .on: "Inkwell starts in the menu bar when you log in."
        case .off: "Inkwell starts when you open it."
        case .needsApproval: "Waiting for you in System Settings > General > Login Items."
        case .unavailable: "Only the installed app can open at login."
        }
    }
}

/// A setting: its name on the left, its controls on the right.
struct SettingRow<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(title).frame(width: 150, alignment: .leading)
            VStack(alignment: .leading, spacing: 6) {
                content
            }
            Spacer(minLength: 0)
        }
        .font(Typography.body)
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
    }
}

// MARK: - Appearance

/// The mode, the dots (each mode keeps its own), your colours in the mode shown, the edge glow and
/// how the ink moves.
struct AppearanceSection: View {
    let theme: GlowTheme

    private var modeName: String { theme.isDark ? "Dark" : "Light" }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Appearance")
            if let failure = theme.failure {
                Text(failure)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
                    .fixedSize(horizontal: false, vertical: true)
            }
            SettingRow(title: "Mode") {
                // Not fixed in size: at its full width (325 pt on macOS 26) beside the row's title it
                // would set the Settings screen's minimum width above the window's.
                Picker("Mode", selection: Binding(get: { theme.settings.mode }, set: { theme.setMode($0) })) {
                    ForEach(GlowTheme.Mode.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }
            SettingRow(title: "Dots") {
                Text("\(modeName) keeps its own choice")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                    ForEach(Glow.presets) { preset in
                        PresetButton(preset: preset, selected: preset.id == theme.preset.id) {
                            theme.setPreset(preset.id)
                        }
                    }
                }
                .frame(maxWidth: 420)
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Dot colours")
            }
            SettingRow(title: "Colours in \(modeName.lowercased()) mode") {
                ColourRow(
                    label: "You", custom: theme.customYou, presetColour: theme.preset.you, presetName: theme.preset.name,
                    set: { theme.setYou($0) })
                ColourRow(
                    label: "Them", custom: theme.customThem, presetColour: theme.preset.them, presetName: theme.preset.name,
                    set: { theme.setThem($0) })
                Text("The orb's lighter shade follows. A colour too dark for dark mode, or too pale for light mode, is adjusted so it stays visible. Text never takes these colours.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            SettingRow(title: "Glow the window's edge") {
                Toggle("Glow the window's edge", isOn: Binding(
                    get: { theme.settings.edgeGlow }, set: { theme.setEdgeGlow($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                Text("While you dictate or a call records")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
            }
            SettingRow(title: "The ink moves") {
                Picker("The ink moves", selection: Binding(get: { theme.settings.motion }, set: { theme.setMotion($0) })) {
                    ForEach(GlowTheme.Motion.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                Text("Following the system, Reduce Motion holds the orb and the edge still.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
            }
            SettingRow(title: "Contrast") {
                Text("With Increase Contrast or Reduce Transparency on (System Settings > Accessibility > Display), cards are solid with stronger borders, and the orb dims behind the text.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A preset: its two dots, each its colour shading into its lighter partner, and its name.
struct PresetButton: View {
    let preset: Glow.Preset
    let selected: Bool
    let pick: () -> Void

    var body: some View {
        Button(action: pick) {
            HStack(spacing: 12) {
                ZStack(alignment: .leading) {
                    dot(preset.you)
                    dot(preset.them).opacity(0.9).offset(x: 16)
                }
                .frame(width: 42, height: 26, alignment: .leading)
                .accessibilityHidden(true)
                Text(preset.name)
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.text)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 46)
            .background(RoundedRectangle(cornerRadius: 14).fill(selected ? PaperPalette.chip : Color.clear))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(selected ? Theme.text : Theme.border, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func dot(_ swatch: Swatch) -> some View {
        let base = GlowColours.rgb(swatch)
        return Circle()
            .fill(RadialGradient(
                colors: [GlowColours.color(GlowColours.partner(base)), GlowColours.color(base)],
                center: UnitPoint(x: 0.35, y: 0.35), startRadius: 0, endRadius: 14))
            .frame(width: 26, height: 26)
    }
}

/// Your colour or theirs in the mode shown: a colour well, whose colour it is, and the way back to
/// the preset's.
private struct ColourRow: View {
    let label: String
    /// "#rrggbb", or nil for the preset's.
    let custom: String?
    let presetColour: Swatch
    let presetName: String
    let set: (String?) -> Void

    var body: some View {
        HStack(spacing: 12) {
            ColorPicker(label, selection: Binding(get: { colour }, set: { set(Self.hex($0)) }), supportsOpacity: false)
                .labelsHidden()
            VStack(alignment: .leading, spacing: 1) {
                Text(label).font(.system(size: 14, weight: .semibold)).foregroundStyle(Theme.text)
                Text(custom == nil ? "\(presetName)'s" : "Your own colour")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
            }
            if custom != nil {
                Button("Use the preset's") { set(nil) }
                    .buttonStyle(.link)
                    .accessibilityLabel("\(label): use the preset's colour")
            }
        }
        .accessibilityElement(children: .contain)
    }

    /// The colour chosen, before it is fitted to the mode.
    private var colour: Color {
        GlowColours.color(GlowColours.parse(custom) ?? GlowColours.rgb(presetColour))
    }

    /// A picked colour as the settings store it, or nil when it cannot be read as sRGB.
    static func hex(_ colour: Color) -> String? {
        guard let srgb = NSColor(colour).usingColorSpace(.sRGB) else { return nil }
        return GlowColours.hex(SIMD3(Double(srgb.redComponent), Double(srgb.greenComponent), Double(srgb.blueComponent)))
    }
}

// MARK: - The language model you bring (Settings > AI)

/// The provider, its server and key, the model, Use and Test, and the local-only switch. The first
/// run's Polish step shows the same rows with `firstRun` (polish): there Use asks polish's consent
/// before choosing, so local-only mode goes off only with it, and the switch is left to Settings.
struct LanguageModelRows: View {
    let cloud: CloudModel
    var firstRun: PolishModel?
    /// The key being typed: sent once on Save key, then cleared. Never kept anywhere else.
    @State private var key = ""
    /// The provider whose key Delete asks about, while it asks.
    @State private var deleting: String?

    /// Use: in the first run, polish's consent step first; in Settings, the choice itself.
    private func use() {
        if let firstRun {
            firstRun.useOwnKey(cloud)
        } else {
            cloud.use()
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SettingRow(title: "Language model") {
                Picker("Language model provider", selection: Binding(
                    get: { cloud.selected ?? "" }, set: { id in
                        key = ""
                        cloud.select(id.isEmpty ? nil : id)
                    })) {
                    Text("None: nothing leaves this Mac").tag("")
                    ForEach(cloud.providers) { Text($0.name).tag($0.id) }
                }
                .labelsHidden()
                .fixedSize()
                .disabled(!cloud.loaded)
                if let provider = cloud.selectedProvider {
                    if provider.customURL {
                        TextField("Server address", text: Binding(get: { cloud.draftBaseURL }, set: { cloud.draftBaseURL = $0 }))
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 340)
                    }
                    HStack(spacing: 8) {
                        // Short enough to fit the field: a longer one was cut off ("Paste a new key to replace t…").
                        SecureField(provider.hasKey ? "Paste a new key" : "Paste your API key", text: $key)
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 340)
                            .accessibilityLabel("API key")
                        Button("Save key") {
                            // Sent once, then gone from the field.
                            let typed = key
                            key = ""
                            cloud.saveKey(typed)
                        }
                        Button(cloud.deleteKeyLabel) { deleting = provider.id }
                            .disabled(!provider.hasKey)
                    }
                    Text(cloud.keyStatus)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                    TextField("Model", text: Binding(get: { cloud.draftModel }, set: { cloud.draftModel = $0 }), prompt: Text(provider.defaultModel))
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 340)
                        .accessibilityLabel("Model")
                }
                let useNote = firstRun == nil ? cloud.useNote : cloud.firstRunUseNote
                Text(useNote)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Button(cloud.useLabel) { use() }
                        .buttonStyle(.borderedProminent)
                        .disabled(!(firstRun?.canUseOwnKey(cloud) ?? cloud.canUse))
                        .accessibilityHint(useNote)
                    Button("Test") { cloud.test() }
                        .disabled(!cloud.canTest)
                        .accessibilityLabel("Test the language model")
                }
                if let message = cloud.testMessage {
                    Text(message)
                        .font(Typography.caption)
                        .foregroundStyle(cloud.testState == .failed ? Theme.alert : Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text(cloud.failure ?? cloud.status)
                    .font(Typography.caption)
                    .foregroundStyle(cloud.failure != nil || cloud.readError != nil ? Theme.alert : Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if firstRun == nil {
                SettingRow(title: "Local only") {
                    Toggle("Local only", isOn: Binding(get: { cloud.localOnly }, set: { cloud.setLocalOnly($0) }))
                        .toggleStyle(.switch)
                        .labelsHidden()
                        .disabled(!cloud.loaded)
                    Text(cloud.localOnly
                        ? "On: nothing leaves this Mac. No language model off it is called, whatever is chosen above."
                        : "Off: a language model you chose off this Mac can be called, by each feature you allow below.")
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text("Polish, voice edit, summaries and Ask use the language model you choose here. With none chosen, they use Apple Intelligence, which runs on this Mac. To bring your own: pick a provider, paste your API key (kept in your keychain for this Mac account, shared by every Inkwell on it, never in Inkwell's files), choose a model and press Use. Test sends the provider your key and one short fixed question, never your words. Nothing else is sent until you turn a feature on below and allow it.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        // The key is the Mac account's, not the library's: Delete says so and asks first.
        // The provider is handed to the actions (presenting:), not read back from @State when
        // they run: the delete is of the key the question named.
        .confirmationDialog(
            cloud.deleteKeyTitle(deleting),
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible,
            presenting: deleting
        ) { id in
            Button("Delete Key", role: .destructive) { cloud.deleteKey(id) }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text(CloudModel.deleteKeyMessage)
        }
    }
}

/// The first run's own key as one choice: Groq's free model, with the homepage's sentence and its
/// console.groq.com link, the key field and Save, and Use, which asks polish's consent before
/// choosing Groq (PolishModel.useOwnKey), so Local only goes off only with it. Another provider or
/// model is under "Other providers or models…": Settings > AI's rows, with the same consent.
struct GroqKeyRows: View {
    let cloud: CloudModel
    let polish: PolishModel
    /// The key being typed: sent once on Save, then cleared. Never kept anywhere else.
    @State private var key = ""
    /// The other providers' rows are shown instead. Set when the rows are made, so a provider
    /// already chosen or picked never flashes Groq's rows first.
    @State private var others: Bool

    init(cloud: CloudModel, polish: PolishModel) {
        self.cloud = cloud
        self.polish = polish
        _others = State(initialValue: cloud.firstRunStartsOnOthers)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if others {
                LanguageModelRows(cloud: cloud, firstRun: polish)
                Button {
                    key = ""
                    cloud.pickGroq()
                    others = false
                } label: {
                    Self.link("Back to Groq's free model")
                }
                .buttonStyle(.plain)
            } else {
                groq
                Button {
                    key = ""
                    others = true
                } label: {
                    Self.link("Other providers or models\u{2026}")
                }
                .buttonStyle(.plain)
            }
        }
        .onAppear {
            // Nothing chosen or picked: Groq goes in the picker (the rows start on it).
            cloud.suggest("groq")
        }
        .onChange(of: cloud.loaded) {
            // Opened before the providers were read: Groq goes in the picker once they are.
            cloud.suggest("groq")
            if cloud.firstRunStartsOnOthers { others = true }
        }
    }

    /// A link in the ink, as the console.groq.com link is (the system's blue is not the app's).
    private static func link(_ title: String) -> some View {
        Text(title).font(Typography.caption).underline().foregroundStyle(Theme.text)
    }

    private var groq: some View {
        let provider = cloud.selectedProvider
        return VStack(alignment: .leading, spacing: 8) {
            Text("Groq's free tier covers ordinary personal use and needs no credit card. Sign in at [console.groq.com](https://console.groq.com), create a key under API Keys and paste it here.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                SecureField("Paste your Groq key", text: $key)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 260)
                    .accessibilityLabel("Groq API key")
                Button("Save") {
                    // Sent once, then gone from the field.
                    let typed = key
                    key = ""
                    cloud.saveKey(typed)
                }
            }
            .disabled(provider?.id != "groq")
            if provider?.id == "groq" {
                Text(cloud.keyStatus)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 8) {
                Button(cloud.useLabel) { polish.useOwnKey(cloud) }
                    .buttonStyle(.borderedProminent)
                    .disabled(!polish.canUseOwnKey(cloud))
                    .accessibilityHint(cloud.firstRunUseNote)
                // The note takes the row's width beside the button and wraps in it, its full
                // height, rather than being clipped when the row is laid out again.
                Text(cloud.firstRunUseNote)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .layoutPriority(1)
            }
            Text(cloud.failure ?? cloud.status)
                .font(Typography.caption)
                .foregroundStyle(cloud.failure != nil || cloud.readError != nil ? Theme.alert : Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
