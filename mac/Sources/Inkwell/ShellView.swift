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
    }
}
