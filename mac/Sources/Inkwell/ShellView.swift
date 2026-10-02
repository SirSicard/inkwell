// The main window's content: the orb behind everything, the sidebar and the route's screen over
// it, and the edge glow on top.
//
//   ┌──────────────────────────────────────────────────┐ ← the edge glow (while live; no clicks)
//   │ sidebar (glass) │ content (the route's screen)    │
//   │ Inkwell         │        ◯ the orb, behind both   │
//   │ Today …         │                                 │
//   │ Settings        │                                 │
//   └──────────────────────────────────────────────────┘
//
// The sidebar is NavigationSplitView's, so macOS 26 draws it as Liquid Glass floating over the orb
// and the toolbar joins it: the OS owns the chrome. Nothing here animates or redraws on its own: a
// view changes only when the store, the theme or the router does, and the orb and the edge draw
// only while something is live (idle, the orb holds one still frame and the edge shows nothing).
import InkBridge
import InkRenderer
import SwiftUI

struct ShellView: View {
    @Bindable var router: Router
    @Environment(CoreStore.self) private var store
    @Environment(ScreenModels.self) private var screens
    @Environment(GlowTheme.self) private var theme
    @Environment(ShellInk.self) private var ink

    private var meetingLive: Bool { store.meeting != nil }

    var body: some View {
        NavigationSplitView {
            Sidebar(router: router, meetingLive: meetingLive)
                .navigationSplitViewColumnWidth(min: 180, ideal: 210, max: 280)
        } detail: {
            RouteScreen(route: router.current)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .navigationTitle(router.current.title)
        }
        .background {
            OrbLayer(
                state: ink.state, palette: theme.palette, placement: Glow.Orb.main, still: theme.motionStill,
                dimmed: theme.solidSurfaces)
                .ignoresSafeArea()
        }
        .overlay {
            EdgeGlowLayer(state: ink.state, palette: theme.palette, on: theme.settings.edgeGlow, still: theme.motionStill)
                .ignoresSafeArea()
                .allowsHitTesting(false)
        }
        .onChange(of: meetingLive) { _, live in
            router.reconcile(meetingLive: live)
        }
        // The first-run state, until it is completed or skipped (dismissing it counts as skipped,
        // except when the app is quitting: OnboardingModel.appQuitting).
        .sheet(isPresented: Binding(
            get: { screens.onboarding.showing },
            set: { if !$0 { screens.onboarding.sheetDismissed() } }
        )) {
            OnboardingView()
        }
    }
}

/// The destinations, grouped, under the wordmark, with Settings at the foot. A native List: rows
/// are VoiceOver elements, arrow keys move the selection, and the system draws the selection.
struct Sidebar: View {
    @Bindable var router: Router
    let meetingLive: Bool
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        List(selection: $router.selection) {
            ForEach(SidebarSection.allCases.filter { $0 != .app }) { section in
                let routes = section.routes(meetingLive: meetingLive)
                if !routes.isEmpty {
                    if let title = section.title {
                        Section(title) { rows(routes) }
                    } else {
                        Section { rows(routes) }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .accessibilityLabel("Sections")
        .safeAreaInset(edge: .top, spacing: 0) {
            Text("Inkwell")
                .font(.system(size: Glow.Size.heading, design: .serif))
                .foregroundStyle(Theme.text)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 18)
                .padding(.top, 4)
                .padding(.bottom, 6)
                .accessibilityAddTraits(.isHeader)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            // Settings at the foot (it is the app section's one route).
            VStack(spacing: 0) {
                ForEach(SidebarSection.app.routes(meetingLive: meetingLive)) { route in
                    FootRow(route: route, selected: router.current == route) { router.open(route) }
                }
            }
            .padding(.horizontal, 10)
            .padding(.bottom, 12)
        }
    }

    private func rows(_ routes: [Route]) -> some View {
        ForEach(routes) { route in
            row(route).tag(route)
        }
    }

    @ViewBuilder
    private func row(_ route: Route) -> some View {
        switch route {
        case .owed:
            // The overdue count; none shows when nothing is late.
            Label(route.title, systemImage: route.symbol)
                .badge(screens.owed.overdueCount(now: Date()))
        case .live:
            HStack(spacing: 8) {
                Label(route.title, systemImage: route.symbol)
                Spacer(minLength: 0)
                PulseDot()
            }
        default:
            Label(route.title, systemImage: route.symbol)
        }
    }
}

/// The sidebar's foot row (Settings): a row like the list's, selected when its screen shows.
private struct FootRow: View {
    let route: Route
    let selected: Bool
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            Label(route.title, systemImage: route.symbol)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8)
                .frame(minHeight: 28)
                .background(RoundedRectangle(cornerRadius: 8).fill(selected ? PaperPalette.chip : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(Theme.text)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Live's dot in the sidebar, in their colour: it pulses while the meeting records (the orb is
/// drawing then anyway), and holds still under Reduce Motion or "Always still".
struct PulseDot: View {
    @Environment(GlowTheme.self) private var theme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var bright = false

    var body: some View {
        let still = reduceMotion || theme.motionStill
        Circle()
            .fill(theme.them)
            .frame(width: 8, height: 8)
            .opacity(still || bright ? 1 : 0.35)
            .animation(still ? nil : .easeInOut(duration: 0.9).repeatForever(autoreverses: true), value: bright)
            .onAppear { bright = true }
            .accessibilityLabel("Recording")
    }
}

/// Record now, under Today's greeting, and why it could not start.
struct RecordControls: View {
    @Environment(CoreStore.self) private var store
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        VStack(alignment: .trailing, spacing: 8) {
            if store.meeting == nil {
                Button {
                    screens.meetings.recordNow()
                } label: {
                    HStack(spacing: 10) {
                        Circle().fill(PaperPalette.recording).frame(width: 9, height: 9)
                            .accessibilityHidden(true)
                        Text("Record now")
                    }
                }
                .buttonStyle(PaperButtonStyle(prominent: true, large: true))
                .accessibilityHint("Records the mic and everything this Mac plays, until you stop it")
                if let failure = screens.meetings.failure(on: .recordNow) {
                    Text(failure)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                        .multilineTextAlignment(.trailing)
                }
            }
        }
    }

    /// Today's status line: the core's state (`meeting.detection`), never the setting, so a
    /// setting the core could not read or a detector that stopped reads as not listening.
    static func listeningText(recording: Bool, listening: Bool?) -> String {
        if recording { return "Recording" }
        switch listening {
        case true?: return "Listening for calls"
        case false?: return "Not listening for calls"
        case nil: return " "
        }
    }
}
