// The four permission cards (Settings and onboarding): what each one allows, whether it is on, and
// what to do when it is not.
//
// The core's probe answers three of them (`permissions.check`); the calendar is EventKit's, read
// here, because the calendar is the shell's (S2.5's "up next"). Nothing polls: a check runs when a
// screen showing the cards appears, when the app becomes active again (the user coming back from
// System Settings, where a permission is revoked), after a request, and once after each launch.
// A check never prompts; a prompt only ever follows the user pressing Allow.
import AppKit
import EventKit
import InkBridge
import os

/// A card.
enum PermissionCard: String, CaseIterable, Identifiable, Sendable {
    /// The microphone.
    case hearYou
    /// System audio: the far end.
    case hearTheOthers
    /// Accessibility: insertion and the dictation key.
    case typeForYou
    /// The calendar.
    case knowYourMeetings

    var id: String { rawValue }

    /// The card's name, in the app's words.
    var title: String {
        switch self {
        case .hearYou: "Hear you"
        case .hearTheOthers: "Hear the others"
        case .typeForYou: "Type for you"
        case .knowYourMeetings: "Know your meetings"
        }
    }

    /// What it is and what it is for, when it is on.
    var detail: String {
        switch self {
        case .hearYou: "Microphone. Needed for dictation and your side of a call."
        case .hearTheOthers: "System audio. Needed for the other side of a call."
        case .typeForYou: "Accessibility. Needed to put dictated text where your cursor is, and for the dictation key."
        case .knowYourMeetings: "Calendar. Names the call and who was in it."
        }
    }

    /// What is lost while it is off.
    var offDetail: String {
        switch self {
        case .hearYou: "The microphone is off. Nothing you say can be written down."
        case .hearTheOthers: "System audio is off. Meetings record only your voice."
        case .typeForYou: "Accessibility is off. Dictation can't type into other apps or hear the dictation key."
        case .knowYourMeetings: "The calendar is off. Meetings are named by the app they were in."
        }
    }

    /// The core's name for it; nil for the calendar, which the shell reads itself.
    var corePermission: PermissionName? {
        switch self {
        case .hearYou: .microphone
        case .hearTheOthers: .systemAudio
        case .typeForYou: .accessibility
        case .knowYourMeetings: nil
        }
    }
}

/// A card's state.
enum CardState: Equatable, Sendable {
    /// Not known yet: the first check has not answered.
    case checking
    /// Granted.
    case allowed
    /// Refused or revoked: the card is red and says what is lost.
    case off
    /// Never asked: Allow shows the system prompt.
    case notAsked
    /// The system gives no answer it can be sure of.
    case unknown

    /// Whether the card shows the alert colour.
    var isAlert: Bool { self == .off }
}

/// The calendar's permission, as EventKit reports it.
protocol CalendarAccess: Sendable {
    /// Now, without prompting.
    func state() -> CardState
    /// Shows the prompt if never asked, else opens the settings pane. `done` runs on the main actor.
    func request(done: @escaping @MainActor @Sendable () -> Void)
}

/// EventKit's answer.
struct EventKitCalendar: CalendarAccess {
    func state() -> CardState {
        switch EKEventStore.authorizationStatus(for: .event) {
        case .fullAccess: .allowed
        // Write-only access cannot read the events that name a call.
        case .writeOnly, .denied, .restricted: .off
        case .notDetermined: .notAsked
        @unknown default: .unknown
        }
    }

    func request(done: @escaping @MainActor @Sendable () -> Void) {
        guard EKEventStore.authorizationStatus(for: .event) == .notDetermined else {
            SystemSettingsPane.open("Privacy_Calendars")
            Task { @MainActor in done() }
            return
        }
        EKEventStore().requestFullAccessToEvents { _, _ in
            Task { @MainActor in done() }
        }
    }
}

/// System Settings > Privacy & Security, at one pane.
enum SystemSettingsPane {
    static func open(_ anchor: String) {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)") {
            NSWorkspace.shared.open(url)
        }
    }
}

/// The cards' states. Main actor; fed by the core's events.
@MainActor
@Observable
final class PermissionsModel {
    private(set) var states: [PermissionCard: CardState] = [:]
    /// A check is on its way (the system-audio probe takes about a second).
    private(set) var checking = false
    /// Screens showing the cards now: activation re-checks only while one is up.
    @ObservationIgnored private var visible = 0
    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let calendar: any CalendarAccess

    init(send: @escaping SendCommand, calendar: any CalendarAccess = EventKitCalendar()) {
        self.send = send
        self.calendar = calendar
    }

    func state(_ card: PermissionCard) -> CardState {
        states[card] ?? .checking
    }

    /// Cards that are off, in order: what the sidebar and the needs-you banner warn about.
    var offCards: [PermissionCard] {
        PermissionCard.allCases.filter { state($0) == .off }
    }

    /// Checks every card now.
    func refresh() {
        states[.knowYourMeetings] = calendar.state()
        checking = true
        send(.permissionsCheck)
    }

    /// A screen with the cards appeared: check, and re-check on each activation while it is up.
    func screenAppeared() {
        visible += 1
        refresh()
    }

    func screenDisappeared() {
        visible = max(0, visible - 1)
    }

    /// The app became active again: the user may be back from System Settings.
    func appBecameActive() {
        if visible > 0 {
            refresh()
        }
    }

    /// The user pressed Allow (or Open Settings) on `card`.
    func request(_ card: PermissionCard) {
        if let permission = card.corePermission {
            send(.permissionRequest(permission))
        } else {
            calendar.request { [weak self] in self?.refresh() }
        }
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .permissionsChecked(let checked):
            checking = false
            states[.hearYou] = Self.card(checked.microphone)
            states[.hearTheOthers] = Self.card(checked.systemAudio)
            states[.typeForYou] = Self.card(checked.accessibility)
        // permission.requested needs nothing: the answer comes from a prompt or System Settings,
        // and the check when the app is active again reads it.
        case .commandFailed(let failed) where failed.command == "permissions.check":
            // No answer: what the cards showed before may no longer be true, and a card must never
            // read allowed (or off) on a guess.
            checking = false
            for card in PermissionCard.allCases where card.corePermission != nil {
                states[card] = .unknown
            }
        default:
            break
        }
    }

    private static func card(_ state: PermissionState) -> CardState {
        switch state {
        case .granted: .allowed
        case .denied: .off
        case .notDetermined: .notAsked
        case .unknown: .unknown
        }
    }
}
