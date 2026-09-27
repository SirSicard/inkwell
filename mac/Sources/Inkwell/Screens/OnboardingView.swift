// The first-run state: a sheet over the window until the user finishes or skips it. What Inkwell
// does, the four permissions (each asked for only when the user presses Allow), whether polish can
// run on this Mac, and how to dictate. Remembered in the core's store (onboarding.done).
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
                Button(onboarding.step == .ready ? "Start" : "Continue") { onboarding.next() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(32)
        .frame(width: 620, height: 520)
        .background(Theme.surface)
        .onAppear { screens.permissions.screenAppeared() }
        .onDisappear { screens.permissions.screenDisappeared() }
    }

    /// Where the user is: dots, never a bar that fills.
    private func stepDots(_ step: OnboardingModel.Step) -> some View {
        HStack(spacing: 8) {
            ForEach(OnboardingModel.Step.allCases, id: \.self) { s in
                Circle()
                    .fill(s == step ? Theme.text : PaperPalette.border)
                    .frame(width: 7, height: 7)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Step \(step.rawValue + 1) of \(OnboardingModel.Step.allCases.count)")
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

    private var polish: some View {
        let polish = screens.polish
        return VStack(alignment: .leading, spacing: 12) {
            Text("Polish").font(Typography.heading).accessibilityAddTraits(.isHeader)
            Text("Polish tidies a dictation's wording before it is typed. It runs on this Mac with Apple Intelligence.")
            Toggle("Polish my words", isOn: Binding(get: { polish.isOn }, set: { polish.setOn($0) }))
                .toggleStyle(.switch)
                .disabled(!polish.canToggle)
            Text(polish.status)
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
        .foregroundStyle(Theme.text)
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
