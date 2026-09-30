// The first-run state: a sheet over the window until the user finishes or skips it. What Inkwell
// does, the four permissions (each asked for only when the user presses Allow), the speech models
// (downloaded only when the user presses Download, and still downloading while the user goes on),
// polish (off, and turned on only through its consent step), and how to dictate. Remembered in
// the core's store (onboarding.done).
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
                case .models: models
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

    /// What will be downloaded (each model not on this Mac, its licence and size, the total, and
    /// where from), and the one Download button that is the user's agreement: nothing is fetched
    /// before it. Continue works at any time; the downloads keep going.
    private var models: some View {
        let catalogue = screens.catalogue
        let offered = catalogue.firstRunModels
        return ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Speech models").font(Typography.heading).accessibilityAddTraits(.isHeader)
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
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(offered, id: \.id) { model in
                            ModelDownloadRow(catalogue: catalogue, model: model, offersDownload: false)
                        }
                    }
                    let total = ModelDownloadRow.size(offered.map(\.sizeBytes).reduce(0, +))
                    HStack(alignment: .firstTextBaseline) {
                        Text("Total: \(total)").font(.system(.body, weight: .semibold))
                        Spacer()
                        if offered.contains(where: { catalogue.download(of: $0) == .notInstalled }) {
                            Button("Download") { catalogue.downloadFirstRunModels() }
                                .accessibilityLabel("Download \(total) from \(CatalogueModel.sources(offered))")
                        }
                    }
                    if !catalogue.asked.isDisjoint(with: offered.map(\.id)) {
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
