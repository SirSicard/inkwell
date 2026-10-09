// The consent step for a feature that sends the user's words to a language model (polish, voice
// edit, summaries and Ask): every place that can turn one on (Settings, the first-run sheet) shows it the same way.
// The switch only asks; this says plainly what the feature sends and where it goes for the model
// it would use, and only Allow sends anything to the core. Cancel, Escape or closing it leaves the
// feature as it was.
import SwiftUI

extension View {
    /// Presents `consent`'s step while it has one pending that `host` asked for.
    func consentStep(_ consent: ConsentModel, host: ConsentModel.Host) -> some View {
        modifier(ConsentStep(consent: consent, host: host))
    }

    /// Polish's consent step.
    func polishConsent(_ polish: PolishModel, host: ConsentModel.Host) -> some View {
        consentStep(polish.consent, host: host)
    }
}

private struct ConsentStep: ViewModifier {
    let consent: ConsentModel
    let host: ConsentModel.Host

    func body(content: Content) -> some View {
        let feature = consent.feature
        let what = ConsentModel.featureName(feature)
        return content.alert(
            ConsentModel.title(feature),
            // Plain closures (the CI runner's Swift 6.3 crashes on some closure forms here).
            isPresented: Binding(get: { consent.pending != nil && consent.host == host }, set: { shown in
                if !shown { consent.cancel() }
            }),
            presenting: consent.pending
        ) { destination in
            Button("Cancel", role: .cancel) { consent.cancel() }
                .accessibilityLabel("Cancel, and leave \(what) off")
            Button(ConsentModel.button(feature, destination)) { consent.allow() }
                .keyboardShortcut(.defaultAction)
                .accessibilityLabel(destination.isOnDevice
                    ? "Turn on \(what) with \(destination.label)"
                    // Meetings send the whole transcript (everyone's words), as the message says.
                    : "Turn on \(what) and send \(feature == .meetings ? "the transcript" : "your words") to \(destination.label)")
        } message: { destination in
            Text(consent.stepMessage(destination))
        }
    }
}
