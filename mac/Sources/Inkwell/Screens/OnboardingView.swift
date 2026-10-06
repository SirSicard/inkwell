// The first-run state: a sheet over the window until the user finishes or skips it. What Inkwell
// does (over the orb, playing a short demo), the four permissions (each asked for only when the
// user presses Allow), the speech models (choices by what they do, the set every job needs always
// included, downloaded only when the user presses Download, and still downloading while the user
// goes on), Inkwell
// 0.2's history (only when there is some to import), the appearance, polish (off, and turned on
// only through its consent step, with Apple's model or the user's own key), and how to dictate,
// with the orb answering the user's voice. Remembered in the core's store (onboarding.done).
import InkBridge
import InkRenderer
import SwiftUI

struct OnboardingView: View {
    @Environment(ScreenModels.self) private var screens
    @Environment(ShellInk.self) private var ink
    /// What the welcome orb shows: a short demo, then still.
    @State private var demo = InkState.idle
    /// The Polish step's own-key rows are open (closed at first: skipping them costs nothing).
    @State private var ownKey = false
    /// The try-it heard nothing (TryItHint): the Ready step says where the microphone is picked.
    @State private var notHearing = false

    /// The sheet's size and margin. Each step fits it without scrolling (OnboardingLayoutTests):
    /// 560 high, not 520, so the Speech models step holds two failures in the core's long words
    /// with their Retry. The main window opens 700 high.
    static let size = CGSize(width: 620, height: 560)
    static let padding: CGFloat = 32
    /// The room a step has: the sheet less its margins, the step dots and the buttons, and the
    /// spacing between them.
    static var stepRoom: CGSize {
        CGSize(width: size.width - 2 * padding, height: size.height - 2 * padding - 7 - 2 * 20 - 28)
    }

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
        .padding(Self.padding)
        .frame(width: Self.size.width, height: Self.size.height)
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
        Self.title(text)
    }

    static func title(_ text: String) -> some View {
        Text(text).font(Typography.heading).accessibilityAddTraits(.isHeader)
    }

    /// How the first run draws its orb: in `orbPalette`'s colours, and never dimmed. The main
    /// window dims its orb for Increase Contrast or Reduce Transparency because text sits over it;
    /// here it sits beside the text, and dimmed to 0.45 it disappeared again.
    static func orbStyle(_ palette: OrbPalette, dark: Bool, solidSurfaces: Bool) -> (palette: OrbPalette, dimmed: Bool) {
        (orbPalette(palette, dark: dark), false)
    }

    /// The first run's orb colours: the theme's, at rest in the other mode's idle colour (the same
    /// violet, a shade made for the other paper). Each mode's own is made to sit quietly behind
    /// text; beside it on the sheet the resting orb was all but invisible: about 1.3:1 at best in
    /// Light ("its orb didn't show"), and in Dark one bright speck with the disc around it under
    /// 2:1. It rests untinted: leaning toward the dots, as the main window's orb does, lightens it
    /// on Light's paper until the disc is gone again (Aurora and Lagoon at 0.15, a few pixels).
    static func orbPalette(_ palette: OrbPalette, dark: Bool) -> OrbPalette {
        var palette = palette
        palette.idle = SIMD3<Float>(GlowColours.rgb(Glow.mode(dark: !dark).idleOrb))
        palette.restTint = 0
        return palette
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
        let style = Self.orbStyle(theme.palette, dark: theme.isDark, solidSurfaces: theme.solidSurfaces)
        return OrbLayer(
            state: state, palette: style.palette, placement: .centred, still: theme.motionStill,
            dimmed: style.dimmed, levels: levels)
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

    private var models: some View {
        FirstRunModelsStep(catalogue: screens.catalogue)
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

    /// The Polish step (FirstRunPolishStep). It fits the step with Groq's guide open
    /// (OnboardingLayoutTests); the scroll view is for larger text than the sheet was measured at.
    private var polish: some View {
        ScrollView {
            FirstRunPolishStep(ownKey: $ownKey)
        }
        .polishConsent(screens.polish, host: .onboarding)
    }

    /// Names the key dictation uses now: the import step can change it from fn. The try-it: the
    /// orb shows what is live (the Drop does too), answering the voice while the key is held.
    /// With no speech model a hold would type nothing: the step says a model is needed, with
    /// Today's download of the recommended set (its size and hosts shown), and no try-it.
    private var ready: some View {
        let dictation = screens.dictation
        let speech = screens.catalogue.speech
        return VStack(alignment: .leading, spacing: 12) {
            title("Ready")
            if let needed = SpeechModels.readyLine(speech) {
                SpeechModelLine(line: needed, lineFont: Typography.body)
                Text("Inkwell lives in the menu bar; this window opens from there.")
            } else {
                Text("Hold \(DictationModel.key(dictation.key)?.name ?? dictation.key), say something, and let go. Inkwell lives in the menu bar; this window opens from there.")
                orb(ink.state, height: 150, live: true)
                Text(ink.state == .dictating ? "Listening…" : "Try it now: the orb answers your voice.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .frame(maxWidth: .infinity)
                if notHearing {
                    Text(TryItHint.text)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.text)
                        .frame(maxWidth: .infinity)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if !screens.permissions.offCards.isEmpty {
                Text("Still off: \(screens.permissions.offCards.map(\.title).joined(separator: ", ")). Settings can turn them on.")
                    .foregroundStyle(Theme.alert)
            }
        }
        .font(Typography.body)
        .foregroundStyle(Theme.text)
        .fixedSize(horizontal: false, vertical: true)
        // A held take that has heard no words in 5 s: one delayed look per take, not a timer.
        .task(id: ink.store.liveDictation?.take) {
            guard ink.store.liveDictation != nil else { return }
            try? await Task.sleep(for: TryItHint.silence)
            if Task.isCancelled { return }
            let hasPartials = ink.store.engines.values.contains { $0.contains { $0.job == .livePartials } }
            if TryItHint.afterHold(phase: ink.store.dictation, live: ink.store.liveDictation, hasPartials: hasPartials) {
                setNotHearing(true)
            }
        }
        .onChange(of: ink.store.lastDictation) { _, outcome in
            if let hint = TryItHint.after(outcome) { setNotHearing(hint) }
        }
    }

    private func setNotHearing(_ on: Bool) {
        guard on != notHearing else { return }
        notHearing = on
        if on { AccessibilityNotification.Announcement(TryItHint.text).post() }
    }
}

/// The first run's try-it hint: when the orb heard nothing, where to pick the microphone. Shown
/// after a take that found no speech or only silence, or 5 s into a take held with no words heard
/// (only where words come live: with no live engine a take shows none until it ends); gone after a
/// take that types something. The first run has no picker of its own (the plan: one place for it).
enum TryItHint {
    static let silence: Duration = .seconds(5)
    static let text = "Not hearing you? Once you're set up, Settings > Sound picks the microphone and tests it."

    /// Whether a take still held after `silence` has heard nothing.
    static func afterHold(phase: CoreStore.DictationPhase, live: CoreStore.LiveDictation?, hasPartials: Bool) -> Bool {
        guard hasPartials, phase == .listening, let live, !live.edit else { return false }
        return live.partial?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true
    }

    /// What a take's end says: show (it heard nothing), hide (it typed something), or nil (no news).
    static func after(_ outcome: CoreStore.DictationOutcome?) -> Bool? {
        switch outcome {
        case .discarded(.silence)?, .discarded(.noSpeech)?, .discarded(.nothingHeard)?: true
        case .inserted?: false
        default: nil
        }
    }
}

/// The Polish step: the switch (Apple's on-device model, where there is one), and the user's own
/// key as one choice, Groq's free model (GroqKeyRows, with how to get the key; Settings > AI's rows
/// behind "Other providers or models…"), where Use asks polish's consent before choosing the
/// provider, so local-only mode goes off only with it.
struct FirstRunPolishStep: View {
    @Environment(ScreenModels.self) private var screens
    @Binding var ownKey: Bool

    var body: some View {
        let polish = screens.polish
        VStack(alignment: .leading, spacing: 12) {
            OnboardingView.title("Polish")
            Text("Polish tidies a dictation's wording before it is typed. It sends what you dictate to a language model, so it stays off unless you turn it on here or in Settings.")
                .fixedSize(horizontal: false, vertical: true)
            Toggle("Polish my words", isOn: Binding(get: { polish.isOn }, set: { polish.setOn($0, from: .onboarding) }))
                .toggleStyle(.switch)
                .disabled(!polish.canToggle)
                .accessibilityHint(polish.status)
            Text(polish.status)
                .font(Typography.caption)
                .foregroundStyle(polish.isProblem ? Theme.alert : Theme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityHidden(true)
            LabelledDisclosure(title: "Use Groq's free model", isExpanded: $ownKey) {
                GroqKeyRows(cloud: screens.cloud, polish: polish)
                    .padding(.top, 6)
            }
        }
        .foregroundStyle(Theme.text)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A disclosure opened and closed from its title's row as well as its arrow: on the Mac a
/// DisclosureGroup answers only its arrow, and a click on the title did nothing.
struct LabelledDisclosure<Content: View>: View {
    let title: String
    @Binding var isExpanded: Bool
    @ViewBuilder let content: Content

    var body: some View {
        DisclosureGroup(isExpanded: $isExpanded) {
            content
        } label: {
            Text(title)
        }
        .disclosureGroupStyle(RowDisclosureStyle())
    }
}

/// The disclosure's row, arrow, title and the width past it, as one plain button: one control
/// for VoiceOver, named by the title, that says whether it is open (SwiftUI has no expanded trait
/// on the Mac, so the value does), and Space presses it. Drawn as the Mac's own disclosure is: a
/// small tertiary chevron that turns down when open, the title beside it, 4 pt above and below
/// the row, and the content under it, not indented (measured against DisclosureGroup's own).
struct RowDisclosureStyle: DisclosureGroupStyle {
    /// The chevron's column and the gap after it, as the system's disclosure lays them out.
    static let chevronWidth: CGFloat = 7
    static let chevronGap: CGFloat = 4.5

    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation { configuration.isExpanded.toggle() }
            } label: {
                HStack(spacing: Self.chevronGap) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(configuration.isExpanded ? 90 : 0))
                        .frame(width: Self.chevronWidth)
                        .accessibilityHidden(true)
                    configuration.label
                }
                .padding(.vertical, 4)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(configuration.isExpanded ? "Expanded" : "Collapsed")
            if configuration.isExpanded {
                configuration.content
            }
        }
    }
}

/// The Speech models step: a choice per outcome, the set every job needs always included, the
/// extras ticked by the user, and one Download carrying the total of what is ticked. Under each
/// choice, its models' names, licences, sizes and hosts. Nothing is fetched before a press;
/// Continue works at any time, and the downloads keep going.
struct FirstRunModelsStep: View {
    let catalogue: CatalogueModel
    /// The extras the user ticked (the set is always included).
    @State private var ticked: Set<CatalogueModel.Choice> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Speech models").font(Typography.heading).accessibilityAddTraits(.isHeader)
            if catalogue.failed {
                Text(CatalogueModel.failedText).foregroundStyle(Theme.alert)
                Button("Try again") { catalogue.requery() }
            } else if !catalogue.listed {
                Text("Checking which models are on this Mac…").foregroundStyle(Theme.secondaryText)
            } else {
                // Every choice on this Mac, before the step or since: the same rows, each reading
                // On this Mac, so the step reads as done rather than empty (it was one line).
                let allHere = catalogue.allOnThisMac
                Text(Self.lead(allHere: allHere))
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(catalogue.choices) { choice in
                        ChoiceRow(catalogue: catalogue, choice: choice, ticked: $ticked)
                    }
                }
                if !allHere { downloadRow }
            }
        }
        .font(Typography.body)
        .foregroundStyle(Theme.text)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The line over the choices: what they are, or, with every choice on this Mac, that there is
    /// nothing to do.
    static func lead(allHere: Bool) -> String {
        allHere
            ? "All set: the models below are on this Mac already, so there is nothing to download."
            : "Inkwell writes down speech with models that run on this Mac. Each is downloaded once, and only when you press Download."
    }

    /// Download, carrying the total of what is ticked, and while downloads run, that they go on.
    private var downloadRow: some View {
        let bytes = catalogue.bytesToDownload(ticked)
        return HStack(alignment: .firstTextBaseline, spacing: 12) {
            if catalogue.downloading {
                Text("You can go on: the downloads keep going, and Settings > Models shows them.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            if bytes > 0 {
                Button(CatalogueModel.downloadTitle(bytes)) { catalogue.download(choices: ticked) }
                    .buttonStyle(.borderedProminent)
                    .accessibilityLabel("\(CatalogueModel.downloadTitle(bytes)) from \(catalogue.downloadSources(ticked))")
            }
        }
    }
}

/// One choice: its box (the set's is ticked and fixed), what it does, its size, and its models;
/// on this Mac, its download's progress, or its failure with Retry, in place of the size.
struct ChoiceRow: View {
    let catalogue: CatalogueModel
    let choice: CatalogueModel.Choice
    @Binding var ticked: Set<CatalogueModel.Choice>

    var body: some View {
        let state = catalogue.state(of: choice)
        // Only a choice still to fetch takes a tick: the set is always in, and one on this Mac or
        // on its way has nothing left to choose.
        let choosable = !choice.isRequired && (state == .available || isFailed(state))
        let isOn = choice.isRequired || !choosable || ticked.contains(choice)
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Toggle(choice.title, isOn: Binding(get: { isOn }, set: { on in
                if on { ticked.insert(choice) } else { ticked.remove(choice) }
            }))
            .toggleStyle(.checkbox)
            .labelsHidden()
            .disabled(!choosable)
            .accessibilityHint(choice.isRequired ? "Always included: every job needs it" : choice.detail ?? "")
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text(choice.title).font(.system(.body, weight: .semibold))
                    Spacer(minLength: 8)
                    trailing(state)
                }
                if let detail = choice.detail {
                    Text(detail)
                        .font(Typography.caption)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(catalogue.entries(choice), id: \.id) { entry in
                    Text(CatalogueModel.facts(entry))
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                }
                if case .failed(let why) = state {
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        Text("Couldn't download it: \(why)")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.alert)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 8)
                        Button("Retry") { catalogue.retry(choice) }
                            .accessibilityLabel("Retry downloading \(choice.title)")
                    }
                }
            }
            .contentShape(Rectangle())
            .onTapGesture {
                // The words tick the box too, as a checkbox's label does.
                guard choosable else { return }
                if ticked.contains(choice) { ticked.remove(choice) } else { ticked.insert(choice) }
            }
        }
        .accessibilityElement(children: .contain)
    }

    private func isFailed(_ state: CatalogueModel.ChoiceState) -> Bool {
        if case .failed = state { true } else { false }
    }

    @ViewBuilder
    private func trailing(_ state: CatalogueModel.ChoiceState) -> some View {
        switch state {
        case .installed:
            Text("On this Mac").font(Typography.caption).foregroundStyle(Theme.secondaryText)
        case .available, .failed:
            Text(catalogue.sizeLabel(choice)).font(Typography.caption).monospacedDigit().foregroundStyle(Theme.secondaryText)
        case .waiting:
            Text("Waiting").font(Typography.caption).foregroundStyle(Theme.secondaryText)
        case .downloading(let progress?):
            HStack(spacing: 8) {
                ProgressView(value: progress.fraction).frame(width: 90)
                Text("\(CatalogueModel.roundedSize(progress.done)) of \(CatalogueModel.roundedSize(progress.total))")
                    .font(Typography.caption)
                    .monospacedDigit()
                    .foregroundStyle(Theme.secondaryText)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Downloading \(choice.title)")
            .accessibilityValue("\(Int(progress.fraction * 100)) percent")
        case .downloading(nil):
            Text("Starting…").font(Typography.caption).foregroundStyle(Theme.secondaryText)
        }
    }
}
