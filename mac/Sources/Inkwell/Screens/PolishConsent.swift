// The consent step for polish: every place that can turn polish on (Settings > AI, the first-run
// sheet) shows it the same way. The toggle only asks; this says plainly what polish sends and where
// it goes for the model polish would use, and only Allow sends anything to the core. Cancel, Escape
// or closing it leaves polish as it was.
import SwiftUI

extension View {
    /// Presents `polish`'s consent step while it has one pending.
    func polishConsent(_ polish: PolishModel) -> some View {
        modifier(PolishConsentStep(polish: polish))
    }
}

private struct PolishConsentStep: ViewModifier {
    let polish: PolishModel

    func body(content: Content) -> some View {
        content.alert(
            PolishModel.consentTitle,
            // Plain closures (the CI runner's Swift 6.3 crashes on some closure forms here).
            isPresented: Binding(get: { polish.pendingConsent != nil }, set: { shown in
                if !shown { polish.cancelConsent() }
            }),
            presenting: polish.pendingConsent
        ) { destination in
            Button("Cancel", role: .cancel) { polish.cancelConsent() }
                .accessibilityLabel("Cancel, and leave polish off")
            Button(PolishModel.consentButton(destination)) { polish.allowConsent() }
                .keyboardShortcut(.defaultAction)
                .accessibilityLabel(destination.isOnDevice
                    ? "Turn on polish with \(destination.label)"
                    : "Turn on polish and send your words to \(destination.label)")
        } message: { destination in
            Text(PolishModel.consentMessage(destination))
        }
    }
}
