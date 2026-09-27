// The main window's content: sidebar, rail, content.
//
//   ┌──────────┬──────┬──────────────────────────────┐
//   │ sidebar  │ rail │ content (the route's screen)  │
//   │ (glass)  │ ink  │ paper / night paper           │
//   └──────────┴──────┴──────────────────────────────┘
//
// The sidebar is NavigationSplitView's, so macOS 26 draws it as Liquid Glass and the toolbar
// joins it: the OS owns the chrome. The rail is where the ink lives beside the content (a paper
// strip; the renderer draws into it from S2.4). Nothing here animates or redraws on its own: a
// view changes only when the store or the router does.
import InkBridge
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
                InkRail()
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

/// The ink rail: a paper strip in both themes. Decorative until the ink is drawn into it, so
/// VoiceOver skips it.
struct InkRail: View {
    var body: some View {
        Rectangle()
            .fill(Theme.inkZone)
            .frame(width: Layout.railWidth)
            .frame(maxHeight: .infinity)
            .accessibilityHidden(true)
    }
}
