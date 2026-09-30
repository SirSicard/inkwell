// The first-run state: a sheet over the window until the user finishes or skips it. What Inkwell
// does, the four permissions (each asked for only when the user presses Allow), Inkwell 0.2's
// history (only when there is some to import), polish (off, and turned on only through its consent
// step), and how to dictate. Remembered in the core's store (onboarding.done).
import SwiftUI

struct OnboardingView: View {
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        let onboarding = screens.onboarding
        VStack(alignment: .leading, spacing: 20) {
            stepDots(onboarding.step)
            Group {
                switch onboarding.step {
                case .welcome: welcome
                case .permissions: permissions
                case .importData: importData
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

    private var welcome: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Inkwell").font(Typography.screenTitle).accessibilityAddTraits(.isHeader)
            Text("Hold fn and speak: your words are typed where your cursor is.")
            Text("In a meeting, Inkwell writes down both sides as they talk, then blots the transcript and lists what you promised.")
            Text("It all happens on this Mac. Nothing is sent anywhere unless you add your own key for a model online.")
                .foregroundStyle(Theme.secondaryText)
        }
        .font(.system(.title3))
        .foregroundStyle(Theme.text)
        .fixedSize(horizontal: false, vertical: true)
    }

    private var permissions: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("What Inkwell needs").font(Typography.heading).accessibilityAddTraits(.isHeader)
            Text("Each is asked for only when you press Allow, and each can be changed later in Settings.")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
            PermissionCards(permissions: screens.permissions)
        }
    }

    /// Shown only while Inkwell 0.2's data is offered: what it left, in words, with Import; after
    /// an import, what became of its dictation key.
    private var importData: some View {
        let dictation = screens.dictation
        return VStack(alignment: .leading, spacing: 12) {
            Text("Your Inkwell 0.2 history").font(Typography.heading).accessibilityAddTraits(.isHeader)
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
            Text("Polish").font(Typography.heading).accessibilityAddTraits(.isHeader)
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

    private var ready: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Ready").font(Typography.heading).accessibilityAddTraits(.isHeader)
            Text("Hold fn, say something, and let go. Inkwell lives in the menu bar; this window opens from there.")
            if !screens.permissions.offCards.isEmpty {
                Text("Still off: \(screens.permissions.offCards.map(\.title).joined(separator: ", ")). Settings can turn them on.")
                    .foregroundStyle(Theme.alert)
            }
        }
        .font(.system(.title3))
        .foregroundStyle(Theme.text)
        .fixedSize(horizontal: false, vertical: true)
    }
}
