// Today's "needs you" banner: what the user must act on, most urgent first. Fed by the silent-
// channel watchdog (a live meeting's side states, and the warnings a meeting raised), the
// permission probes (permissions.check), and the library's record of recent meetings that kept no
// far end: Blotter recorded one side for four weeks without saying so, and this banner is where
// Inkwell says so.
import Foundation
import InkBridge

struct NeedsYouItem: Identifiable, Equatable, Sendable {
    /// What its button does.
    enum Action: Equatable, Sendable {
        /// Opens System Settings at a privacy pane.
        case openSettings(SettingsPane)
        /// Dismisses a one-off notice.
        case dismiss(CoreStore.Notice.ID)
    }

    /// A pane of System Settings > Privacy & Security.
    enum SettingsPane: String, Equatable, Sendable {
        case microphone = "Privacy_Microphone"
        case systemAudio = "Privacy_AudioCapture"
        case accessibility = "Privacy_Accessibility"

        var url: URL? {
            URL(string: "x-apple.systempreferences:com.apple.preference.security?\(rawValue)")
        }
    }

    let id: String
    let title: String
    let detail: String
    let actionTitle: String
    let action: Action
}

enum NeedsYou {
    /// Everything that needs the user now, most urgent first.
    static func items(
        permissions: PermissionsChecked?,
        farSilentMeetings: Int64,
        farSilentSince: Date?,
        meeting: CoreStore.LiveMeeting?,
        notices: [CoreStore.Notice],
        now: Date,
        calendar: Calendar
    ) -> [NeedsYouItem] {
        var items: [NeedsYouItem] = []
        let allowAudio = NeedsYouItem.Action.openSettings(.systemAudio)

        // The meeting going on now, as the watchdog judges each side.
        if let meeting {
            switch meeting.sides[.far] {
            case .zeros?:
                items.append(.init(
                    id: "live-far", title: "Inkwell can't hear the other side of this call",
                    detail: "The other side is arriving as silence. System audio may be off for Inkwell.",
                    actionTitle: "Allow system audio", action: allowAudio))
            case .stopped?:
                items.append(.init(
                    id: "live-far", title: "The other side stopped coming through",
                    detail: "No audio has arrived from the call for a while. This meeting may keep only your voice.",
                    actionTitle: "Check system audio", action: allowAudio))
            default:
                break
            }
            switch meeting.sides[.mic] {
            case .zeros?, .stopped?:
                items.append(.init(
                    id: "live-mic", title: "Inkwell can't hear you in this call",
                    detail: "Your microphone is sending silence. This meeting may keep only the other side.",
                    actionTitle: "Check the microphone", action: .openSettings(.microphone)))
            default:
                break
            }
        }

        // Permissions.
        let since = farSilentSince.map { $0.formatted(Date.FormatStyle(timeZone: calendar.timeZone)
            .locale(calendar.locale ?? .current).day().month(.abbreviated)) }
        if permissions?.systemAudio == .denied {
            let detail = since.map { "System audio has been off since \($0), so your meetings kept only your own voice." }
                ?? "System audio is off, so meetings keep only your own voice."
            items.append(.init(
                id: "perm-system-audio", title: "Inkwell can't hear the other side of your calls",
                detail: detail, actionTitle: "Allow system audio", action: allowAudio))
        } else if farSilentMeetings > 0 {
            let count = farSilentMeetings == 1 ? "Your last meeting" : "Your last \(farSilentMeetings) meetings"
            let from = since.map { " (since \($0))" } ?? ""
            items.append(.init(
                id: "far-silent", title: "Inkwell didn't hear the other side of your calls",
                detail: "\(count)\(from) kept only your own voice. Check that system audio is allowed.",
                actionTitle: "Allow system audio", action: allowAudio))
        }
        if permissions?.microphone == .denied {
            items.append(.init(
                id: "perm-mic", title: "Inkwell can't hear you",
                detail: "Microphone access is off, so dictation and meetings record nothing from you.",
                actionTitle: "Allow the microphone", action: .openSettings(.microphone)))
        }
        if permissions?.accessibility == .denied {
            items.append(.init(
                id: "perm-ax", title: "Dictation can't type for you",
                detail: "Accessibility is off, so the dictation key and typing into other apps don't work.",
                actionTitle: "Allow Accessibility", action: .openSettings(.accessibility)))
        }

        // One-off notices the core sent: what went wrong in a meeting or with the dictation key.
        for notice in notices.reversed() {
            guard let (title, detail) = describe(notice.kind) else { continue }
            items.append(.init(
                id: "notice-\(notice.id)", title: title, detail: detail, actionTitle: "Dismiss",
                action: .dismiss(notice.id)))
        }
        return items
    }

    /// The notices that need the user, in words; nil for the rest (the screens that own them show
    /// them: Settings, Live).
    static func describe(_ kind: CoreStore.Notice.Kind) -> (String, String)? {
        switch kind {
        case .meetingWarning(.capturedOnlyZeros):
            ("A meeting recorded only silence on one side", "One side of the last meeting arrived as digital silence: its permission was probably off.")
        case .meetingWarning(.bluetoothMicOnlyZeros):
            ("Your headset's microphone sent only silence", "Bluetooth headset mics can go silent in calls. Inkwell records the built-in mic instead when it can.")
        case .meetingWarning(.nothingCaptured):
            ("The last meeting recorded nothing", "No audio reached Inkwell from either side.")
        case .meetingWarning(.farEndQuietWhileYouSpeak):
            ("The other side went quiet while you spoke", "Their audio may not be reaching Inkwell.")
        case .meetingFailed, .meetingCaptureFailed, .meetingWorkerFailed:
            ("The last meeting stopped early", "What was recorded up to then is kept.")
        case .hotkeyLost:
            ("The dictation key stopped working", "macOS stopped sending it to Inkwell. Check Accessibility in System Settings.")
        default:
            nil
        }
    }
}
