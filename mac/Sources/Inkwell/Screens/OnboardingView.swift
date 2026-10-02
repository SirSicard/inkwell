// The first-run state: a sheet over the window until the user finishes or skips it. What Inkwell
// does (over the orb, playing a short demo), the four permissions (each asked for only when the
// user presses Allow), the speech models (a recommended set and optional extras, each downloaded
// only when the user presses Download, and still downloading while the user goes on), Inkwell
// 0.2's history (only when there is some to import), the appearance, polish (off, and turned on
// only through its consent step), and how to dictate, with the orb answering the user's voice.
// Remembered in the core's store (onboarding.done).
import InkRenderer
import SwiftUI

struct OnboardingView: View {
    @Environment(ScreenModels.self) private var screens
    @Environment(ShellInk.self) private var ink
    /// What the welcome orb shows: a short demo, then still.
    @State private var demo = InkState.idle

    var body: some View {
        let onboarding = screens.onboarding
        VStack(alignment: .leading, spacing: 20) {
            stepDots(onboarding.step)
            Group {
                switch onboarding.step {
                case .welcome: welcome
                case .permissions: permissions
                case .models: models
                case .importData: importData
                case .appearance: appearance
                case .polish: polish
                case .ready: ready
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            HStack {
                if onboarding.step != .ready {
                    Button("Skip") { onboarding.finish() }
                        .accessibilityHint("Closes this; Settings has everything here")
                }
                Spacer()
                if onboarding.step != .welcome {
                    Button("Back") { onboarding.back() }
                }
                Button(continueTitle(onboarding.step)) { onboarding.next() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(32)
        .frame(width: 620, height: 520)
        .background(Theme.surface)
        .onAppear { screens.permissions.screenAppeared() }
        .onDisappear { screens.permissions.screenDisappeared() }
    }

    /// The import step moves on without importing: Not now, until something came over.
    private func continueTitle(_ step: OnboardingModel.Step) -> String {
        switch step {
        case .ready: "Start"
        case .importData where screens.import02.imported == nil: "Not now"
        default: "Continue"
        }
    }

    /// Where the user is: dots, never a bar that fills. Only the steps shown.
    private func stepDots(_ step: OnboardingModel.Step) -> some View {
        let steps = screens.onboarding.steps
        return HStack(spacing: 8) {
            ForEach(steps, id: \.self) { s in
                Circle()
                    .fill(s == step ? Theme.text : PaperPalette.border)
                    .frame(width: 7, height: 7)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Step \((steps.firstIndex(of: step) ?? 0) + 1) of \(steps.count)")
    }

    /// Every step's title, the welcome's too: one scale.
    private func title(_ text: String) -> some View {
        Text(text).font(Typography.heading).accessibilityAddTraits(.isHeader)
    }

    /// The orb in the theme's colours: the welcome's demo, the ready step's try-it.
    private func orb(_ state: InkState, height: CGFloat, live: Bool) -> some View {
        let theme = screens.theme
        let levels: @MainActor @Sendable () -> InkLevels
        if live {
            levels = { ShellInk.liveLevels() }
        } else {
            levels = { .silent }
        }
        return OrbLayer(
            state: state, palette: theme.palette, placement: .centred, still: theme.motionStill,
            dimmed: theme.solidSurfaces, levels: levels)
            .frame(maxWidth: .infinity)
            .frame(height: height)
            .accessibilityHidden(true)
    }

    /// Names the key dictation uses now (after Back, the import step may have changed it). The orb
    /// plays a short demo, a dictation then a call, and settles: nothing moves after it.
    private var welcome: some View {
        let dictation = screens.dictation
        return VStack(alignment: .leading, spacing: 12) {
            orb(demo, height: 150, live: false)
            title("Inkwell")
            Text("Hold \(DictationModel.key(dictation.key)?.name ?? dictation.key) and speak: your words are typed where your cursor is.")
            Text("In a meeting, Inkwell writes down both sides as they talk, then blots the transcript and lists what you promised.")
            Text("It all happens on this Mac. Nothing is sent anywhere unless you add your own key for a model online.")
                .foregroundStyle(Theme.secondaryText)
        }
        .font(Typography.body)
        .foregroundStyle(Theme.text)
        .fixedSize(horizontal: false, vertical: true)
        .task {
            demo = .idle
            for (state, seconds) in [(InkState.dictating, 0.6), (.meeting, 3.5), (.idle, 4.5)] {
                try? await Task.sleep(for: .seconds(seconds))
                if Task.isCancelled { return }
                demo = state
            }
        }
    }

    /// Light, dark or the system's, and a row of dot presets for the mode shown.
    private var appearance: some View {
        let theme = screens.theme
        return VStack(alignment: .leading, spacing: 12) {
            title("Appearance")
            Picker("Mode", selection: Binding(get: { theme.settings.mode }, set: { theme.setMode($0) })) {
                ForEach(GlowTheme.Mode.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                ForEach(Glow.presets) { preset in
                    PresetButton(preset: preset, selected: preset.id == theme.preset.id) {
                        theme.setPreset(preset.id)
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Dot colours")
            Text("You can change this and pick your own colours in Settings > Appearance.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
        .foregroundStyle(Theme.text)
    }

    private var permissions: some View {
        VStack(alignment: .leading, spacing: 12) {
            title("What Inkwell needs")
            Text("Each is asked for only when you press Allow, and each can be changed later in Settings.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
            PermissionCards(permissions: screens.permissions)
        }
    }

    /// What will be downloaded (each model not on this Mac, its licence and size, and where from):
    /// the recommended set with its total and the one Download that is the user's agreement to it,
    /// then the optional extras, each with what it adds and its own Download. Nothing is fetched
    /// before a press. Continue works at any time; the downloads keep going.
    private var models: some View {
        let catalogue = screens.catalogue
        let offered = catalogue.firstRunModels
        let recommended = catalogue.firstRunRecommended
        let extras = catalogue.firstRunExtras
        return ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                title("Speech models")
                if catalogue.failed {
                    Text(CatalogueModel.failedText).foregroundStyle(Theme.alert)
                    Button("Try again") { catalogue.requery() }
                } else if !catalogue.listed {
                    Text("Checking which models are on this Mac…").foregroundStyle(Theme.secondaryText)
                } else if offered.isEmpty {
                    Text("Every model Inkwell uses is on this Mac already.")
                } else {
                    Text("Inkwell turns speech into text with models that run on this Mac. They are downloaded once, from \(CatalogueModel.sources(offered)), and only when you press Download.")
                        .fixedSize(horizontal: false, vertical: true)
                    Paper.Eyebrow(text: "Recommended")
                    if recommended.isEmpty {
                        Text("The recommended models are on this Mac already.")
                    } else {
                        Text("Voice detection and Parakeet: enough for dictation, the live words and meeting transcripts.")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(recommended, id: \.id) { model in
                                ModelDownloadRow(catalogue: catalogue, model: model, offersDownload: false)
                            }
                        }
                        let total = ModelDownloadRow.size(recommended.map(\.sizeBytes).reduce(0, +))
                        HStack(alignment: .firstTextBaseline) {
                            Text("Total: \(total)").font(.system(.body, weight: .semibold))
                            Spacer()
                            if recommended.contains(where: { catalogue.download(of: $0) == .notInstalled }) {
                                Button("Download") { catalogue.downloadRecommended() }
                                    .accessibilityLabel("Download the recommended models, \(total), from \(CatalogueModel.sources(recommended))")
                            }
                        }
                    }
                    if !extras.isEmpty {
                        Paper.Eyebrow(text: "Optional")
                            .padding(.top, 6)
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(extras, id: \.id) { model in
                                ModelDownloadRow(catalogue: catalogue, model: model, offersDownload: true)
                                if let adds = CatalogueModel.adds(model.id) {
                                    Text(adds)
                                        .font(Typography.caption)
                                        .foregroundStyle(Theme.secondaryText)
                                        .fixedSize(horizontal: false, vertical: true)
                                        .padding(.bottom, 4)
                                }
                            }
                        }
                    }
                    if catalogue.downloading {
                        Text("You can go on: the downloads keep going, and Settings > Models shows them.")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.secondaryText)
                    }
                }
            }
            .foregroundStyle(Theme.text)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Shown only while Inkwell 0.2's data is offered: what it left, in words, with Import; after
    /// an import, what became of its dictation key.
    private var importData: some View {
        let dictation = screens.dictation
        return VStack(alignment: .leading, spacing: 12) {
            title("Your Inkwell 0.2 history")
            Import02Card(model: screens.import02)
            if screens.import02.imported != nil {
                ImportKeyNoteView(model: screens.importNote, currentKey: DictationModel.key(dictation.key)?.name ?? dictation.key)
            }
        }
        .foregroundStyle(Theme.text)
    }

    private var polish: some View {
        let polish = screens.polish
        return VStack(alignment: .leading, spacing: 12) {
            title("Polish")
            Text("Polish tidies a dictation's wording before it is typed. It sends what you dictate to a language model, so it stays off unless you turn it on here or in Settings.")
            Toggle("Polish my words", isOn: Binding(get: { polish.isOn }, set: { polish.setOn($0, from: .onboarding) }))
                .toggleStyle(.switch)
                .disabled(!polish.canToggle)
                .accessibilityHint(polish.status)
            Text(polish.status)
                .font(Typography.caption)
                .foregroundStyle(polish.isProblem ? Theme.alert : Theme.secondaryText)
                .accessibilityHidden(true)
        }
        .foregroundStyle(Theme.text)
        .polishConsent(polish, host: .onboarding)
    }

    /// Names the key dictation uses now: the import step can change it from fn. The try-it: the
    /// orb shows what is live (the Drop does too), answering the voice while the key is held.
    private var ready: some View {
        let dictation = screens.dictation
        return VStack(alignment: .leading, spacing: 12) {
            title("Ready")
            Text("Hold \(DictationModel.key(dictation.key)?.name ?? dictation.key), say something, and let go. Inkwell lives in the menu bar; this window opens from there.")
            orb(ink.state, height: 150, live: true)
            Text(ink.state == .dictating ? "Listening…" : "Try it now: the orb answers your voice.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
                .frame(maxWidth: .infinity)
            if !screens.permissions.offCards.isEmpty {
                Text("Still off: \(screens.permissions.offCards.map(\.title).joined(separator: ", ")). Settings can turn them on.")
                    .foregroundStyle(Theme.alert)
            }
        }
        .font(Typography.body)
        .foregroundStyle(Theme.text)
        .fixedSize(horizontal: false, vertical: true)
    }
}
