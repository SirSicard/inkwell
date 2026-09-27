// The main window's content: sidebar, rail, content.
//
//   ┌──────────┬──────┬──────────────────────────────┐
//   │ sidebar  │ rail │ content (the route's screen)  │
//   │ (glass)  │ ink  │ paper / night paper           │
//   └──────────┴──────┴──────────────────────────────┘
//
// The sidebar is NavigationSplitView's, so macOS 26 draws it as Liquid Glass and the toolbar
// joins it: the OS owns the chrome. The rail is where the ink lives beside the content: on Today
// it widens into the full ink zone with the INKWELL wordmark, everywhere else it is a 56 pt paper
// strip. Nothing here animates or redraws on its own: a view changes only when the store or the
// router does, and the ink draws only while something is live.
import InkBridge
import InkRenderer
import SwiftUI

struct ShellView: View {
    @Bindable var router: Router
    @Environment(CoreStore.self) private var store
    @Environment(ScreenModels.self) private var screens

    private var meetingLive: Bool { store.meeting != nil }

    var body: some View {
        NavigationSplitView {
            Sidebar(router: router, meetingLive: meetingLive)
                .navigationSplitViewColumnWidth(min: 180, ideal: 210, max: 280)
        } detail: {
            HStack(spacing: 0) {
                InkRail(wide: router.current == .today)
                RouteScreen(route: router.current)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Theme.surface)
            }
            .navigationTitle(router.current.title)
        }
        .tint(Theme.accent)
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

/// The destinations, grouped. A native List: rows are VoiceOver elements, arrow keys move the
/// selection, and the system draws the selection in the accent colour.
struct Sidebar: View {
    @Bindable var router: Router
    let meetingLive: Bool

    var body: some View {
        List(selection: $router.selection) {
            ForEach(SidebarSection.allCases) { section in
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
    }

    private func rows(_ routes: [Route]) -> some View {
        ForEach(routes) { route in
            Label(route.title, systemImage: route.symbol)
                .tag(route)
        }
    }
}

/// The ink rail: paper in both themes, with the ink drawn into it. On Today it is the full ink
/// zone with the wordmark knocked out of the ink; elsewhere a narrow strip. Decorative: the state
/// it shows is spoken by the Drop and the screens, so VoiceOver skips it.
struct InkRail: View {
    /// Today's full ink zone rather than the strip.
    let wide: Bool
    @Environment(ShellInk.self) private var ink

    var body: some View {
        InkZone(state: ink.state, showsWordmark: wide)
            .frame(width: wide ? Layout.inkZoneWidth : Layout.railWidth)
            .frame(maxHeight: .infinity)
            .background(Theme.inkZone)
            .accessibilityHidden(true)
            .overlay(alignment: .bottom) {
                if wide {
                    RecordControls().padding(22)
                }
            }
    }
}

/// Today's foot of the ink zone (the canvas): whether Inkwell listens for calls, and Record now.
struct RecordControls: View {
    @Environment(CoreStore.self) private var store
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(listening)
                Text("Hold fn to dictate").foregroundStyle(Theme.secondaryText)
            }
            .font(Typography.timestamp)
            .foregroundStyle(PaperPalette.quiet)
            if store.meeting == nil {
                Button {
                    screens.meetings.recordNow()
                } label: {
                    HStack(spacing: 8) {
                        Circle().fill(PaperPalette.recording).frame(width: 8, height: 8)
                            .accessibilityHidden(true)
                        Text("Record now")
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(PaperButtonStyle(prominent: true))
                .accessibilityHint("Records the mic and everything this Mac plays, until you stop it")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var listening: String {
        Self.listeningText(recording: store.meeting != nil, listening: store.listening)
    }

    /// What the foot of Today says: the core's state (`meeting.detection`), never the setting, so
    /// a setting the core could not read or a detector that stopped reads as not listening.
    static func listeningText(recording: Bool, listening: Bool?) -> String {
        if recording { return "Recording" }
        switch listening {
        case true?: return "Listening for meetings"
        case false?: return "Not listening for meetings"
        case nil: return " "
        }
    }
}
