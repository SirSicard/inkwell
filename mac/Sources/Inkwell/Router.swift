// The window's destinations: what the sidebar lists, in which section, and the screen each one
// shows. A screen step adds or fills a route here and nowhere else in the shell.
//
// The sidebar is a native SwiftUI List, so every row is a VoiceOver element with a working press
// action (the 0.2 app's custom tabs had none). Selecting a row is the whole navigation model:
// there is no second history to keep in step.
import InkBridge
import SwiftUI

/// A sidebar destination.
enum Route: String, CaseIterable, Identifiable, Hashable, Sendable {
    case today
    case library
    case owed
    case live
    case settings

    var id: String { rawValue }

    /// The row's name, which VoiceOver reads.
    var title: String {
        switch self {
        case .today: "Today"
        case .library: "Library"
        case .owed: "Owed"
        case .live: "Live"
        case .settings: "Settings"
        }
    }

    /// The row's SF Symbol.
    var symbol: String {
        switch self {
        case .today: "sun.max"
        case .library: "books.vertical"
        case .owed: "checklist"
        case .live: "waveform"
        case .settings: "gearshape"
        }
    }

    var section: SidebarSection {
        switch self {
        case .today, .library, .owed: .main
        case .live: .recording
        case .settings: .app
        }
    }

    /// Whether the sidebar lists it now. Live exists only while a meeting is recorded or blotted.
    func isListed(meetingLive: Bool) -> Bool {
        self == .live ? meetingLive : true
    }
}

/// The sidebar's groups, in order.
enum SidebarSection: CaseIterable, Identifiable, Sendable {
    case main
    case recording
    case app

    var id: Self { self }

    /// The group's heading; nil for none.
    var title: String? {
        switch self {
        case .main: nil
        case .recording: "While recording"
        case .app: nil
        }
    }

    /// Its routes the sidebar lists now, in declaration order.
    func routes(meetingLive: Bool) -> [Route] {
        Route.allCases.filter { $0.section == self && $0.isListed(meetingLive: meetingLive) }
    }
}

/// Which screen the window shows. Main actor; the screens and the sidebar share one.
@MainActor
@Observable
final class Router {
    /// The selected route. Optional because a List selection can be cleared; the window then
    /// shows Today.
    var selection: Route? = .today

    /// The route whose screen is shown.
    var current: Route { selection ?? .today }

    /// Shows `route`.
    func open(_ route: Route) {
        selection = route
    }

    /// Keeps the selection on a listed row: when the meeting ends, Live leaves the sidebar, and a
    /// window left showing it goes to Today.
    func reconcile(meetingLive: Bool) {
        if let selection, !selection.isListed(meetingLive: meetingLive) {
            self.selection = .today
        }
    }
}

/// The screen for a route.
struct RouteScreen: View {
    let route: Route

    var body: some View {
        switch route {
        // Today and Library show the placeholder until S2.5 lands them.
        case .today, .library:
            PlaceholderScreen(route: route)
        case .owed:
            OwedScreen()
        case .live:
            LiveScreen()
        case .settings:
            SettingsScreen()
        }
    }
}

/// A route whose screen is not built yet: its title, and whether the core is running.
struct PlaceholderScreen: View {
    let route: Route
    @Environment(CoreStore.self) private var store

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(route.title)
                .font(Typography.screenTitle)
                .foregroundStyle(Theme.text)
                .accessibilityAddTraits(.isHeader)
            Text("This screen is not built yet.")
                .font(Typography.body)
                .foregroundStyle(Theme.secondaryText)
            Text(CoreStatusText.line(for: store.status))
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
            Spacer()
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

/// The core's status in words (the status menu and the placeholder screens).
enum CoreStatusText {
    static func line(for status: CoreStore.Status) -> String {
        switch status {
        case .starting: "Starting…"
        case .ready(let version): "Ready (core \(version))"
        case .stopped: "Stopped"
        case .failed(let reason): reason
        }
    }
}
