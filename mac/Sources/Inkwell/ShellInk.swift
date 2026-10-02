// The ink the shell shows: one state for every surface that draws it (the Drop, the window's orb and
// its edge glow, the menu-bar dot), read from the core's events through the store. The core pushes, the ink
// follows: nothing here polls.
//
//   dictation listening or transcribing   dictating
//   a meeting recording                   meeting, or problem while the far end delivers nothing
//                                         (or the system-audio probe says it is off)
//   a meeting stopped, final pass running blotting
//   nothing live                          idle (the Drop may still show an offer to record)
import InkBridge
import InkRenderer
import Observation
import SwiftUI

@MainActor
@Observable
final class ShellInk {
    @ObservationIgnored let store: CoreStore
    /// The permission cards: a system-audio probe that says "off" during a meeting is a problem the
    /// Drop shows at once, before the watchdog has heard ten seconds of silence.
    @ObservationIgnored let permissions: PermissionsModel?
    /// The meeting commands: a Drop answer that failed is said in the Drop.
    @ObservationIgnored let meetings: MeetingModel?
    /// A state held whatever the core says: the shell budget's live phase (INK_MEASURE=live) and
    /// the Drop's focus check (INK_DROP_DEMO). Nil in ordinary use.
    var held: InkState?

    init(store: CoreStore, permissions: PermissionsModel? = nil, meetings: MeetingModel? = nil) {
        self.store = store
        self.permissions = permissions
        self.meetings = meetings
    }

    /// Whether the system-audio probe says it is off.
    private var systemAudioOff: Bool {
        permissions?.state(.hearTheOthers) == .off
    }

    /// What the ink shows now.
    var state: InkState {
        held ?? Self.state(meeting: store.meeting, dictation: store.dictation, systemAudioOff: systemAudioOff)
    }

    /// What the Drop says beside it.
    var dropText: DropText {
        if held != nil {
            return DropText.for(state, dictation: store.dictation, live: store.liveDictation)
        }
        return DropText.for(
            state, dictation: store.dictation, live: store.liveDictation, meeting: store.meeting,
            offer: store.offer, systemAudioOff: systemAudioOff, failure: meetings?.failure(on: .drop))
    }

    /// Whether the Drop shows: something is live, or the core offers to record a call.
    var dropShows: Bool {
        state.isLive || (held == nil && store.offer != nil)
    }

    /// The state for what is live. A meeting outranks a dictation. Only the far end sets the
    /// problem state, because the problem ink shows the far end's drop gone dead; a silent mic
    /// shows as your own drop lying still. `systemAudioOff`: the probe says the far end cannot be
    /// heard, which the watchdog would only notice after ten seconds of silence.
    nonisolated static func state(
        meeting: CoreStore.LiveMeeting?, dictation: CoreStore.DictationPhase, systemAudioOff: Bool = false
    ) -> InkState {
        if let meeting {
            if meeting.stopping { return .blotting }
            if let far = meeting.sides[.far], far != .ok { return .problem }
            if systemAudioOff { return .problem }
            return .meeting
        }
        return dictation == .idle ? .idle : .dictating
    }

    /// The live levels, read by each ink at vsync: your mic's bands (ink_bands_read) and, during a
    /// meeting, the far end's (ink_far_bands_read), so each drop pulses with its own side.
    static func liveLevels() -> InkLevels {
        let near = InkSession.bands()
        let far = InkSession.farBands()
        return InkLevels(
            near: InkLevels.level(low: near.low, mid: near.mid, high: near.high),
            far: InkLevels.level(low: far.low, mid: far.mid, high: far.high))
    }
}

/// The Drop's two lines, and the buttons it offers. While a dictation is held the second line holds
/// its live words; during a meeting, the latest line said.
struct DropText: Equatable, Sendable {
    /// How the Drop colours the title and its border.
    enum Tone: Equatable, Sendable {
        case plain
        /// A recording: the title in seal red, as a recording light.
        case recording
        /// Something needs the user: the title and the border in seal red.
        case alert
    }

    /// A button on the Drop.
    enum Action: Equatable, Sendable {
        /// Record the call the core offered (meeting.start with its app).
        case record(app: String)
        /// "Not this one" (meeting.dismiss).
        case dismiss(app: String)
        /// Ask for system audio (as the Settings card does).
        case allowSystemAudio

        var title: String {
            switch self {
            case .record: "Record this call"
            case .dismiss: "Not this one"
            case .allowSystemAudio: "Allow system audio"
            }
        }
    }

    var title: String
    var detail: String
    var tone = Tone.plain
    var actions: [Action] = []
    /// The detail is the live words of a take being held: its end matters (the head is cut, not
    /// the tail), and its newest words are still wet.
    var liveWords = false

    /// What the Drop says for what is going on. A consent offer shows only while nothing is live.
    /// The consent line is honest about what recording does: both sides are kept on this Mac, and
    /// the others should be told (the app ships consent tooling; it never claims to be unseen).
    /// `failure`: a Drop answer that failed, in words; the offer stays, so it can be answered
    /// again. A dictation shows as `for(_:dictation:live:)` has it.
    static func `for`(
        _ state: InkState, dictation: CoreStore.DictationPhase, live: CoreStore.LiveDictation? = nil,
        meeting: CoreStore.LiveMeeting?, offer: CoreStore.Offer?, systemAudioOff: Bool,
        failure: String? = nil
    ) -> DropText {
        switch state {
        case .idle:
            guard let offer else { return DropText.for(state, dictation: dictation, live: live) }
            return DropText(
                title: "\(offer.appName) opened the microphone",
                detail: failure ?? "Recording keeps both sides on this Mac. Tell the others you are recording.",
                tone: failure == nil ? .plain : .alert,
                actions: [.record(app: offer.app), .dismiss(app: offer.app)])
        case .meeting:
            guard let meeting else { return DropText.for(state, dictation: dictation, live: live) }
            let source = meeting.appName ?? meeting.title
            let latest = meeting.finals.last?.text.trimmingCharacters(in: .whitespacesAndNewlines)
            // Said until the first line arrives (Live keeps saying it): other apps' sound is in
            // this recording.
            let waiting = meeting.farEndFallback
                ? "Inkwell couldn't hear \(meeting.appName ?? "the call") alone, so it is recording everything this Mac plays"
                : "Recording this meeting"
            return DropText(
                title: ["● REC", source].compactMap { $0 }.joined(separator: " · "),
                detail: latest.flatMap { $0.isEmpty ? nil : $0 } ?? waiting,
                tone: .recording)
        case .problem:
            let far = meeting?.sides[.far]
            let title = switch far {
            case .zeros?: "The other side is silent"
            case .stopped?: "The other side stopped"
            default: systemAudioOff ? "System audio is off" : "Far end silent"
            }
            let detail = systemAudioOff || far == .zeros
                ? "System audio is off, so only your voice is being recorded."
                : "Nothing is arriving from the call. Only your voice may be recorded."
            return DropText(title: title, detail: detail, tone: .alert, actions: [.allowSystemAudio])
        case .blotting:
            return DropText(title: "Blotting · final pass", detail: meeting?.title ?? meeting?.appName ?? "The final pass")
        case .dictating:
            return DropText.for(state, dictation: dictation, live: live)
        }
    }

    /// How many of the newest live words are shown wet (italic, muted), as on the canvas.
    static let wetWords = 2

    static func `for`(_ state: InkState, dictation: CoreStore.DictationPhase, live: CoreStore.LiveDictation? = nil) -> DropText {
        switch state {
        case .idle:
            DropText(title: "", detail: "")
        case .dictating:
            dictating(dictation, live: live)
        case .meeting:
            DropText(title: "● REC", detail: "Recording this meeting", tone: .recording)
        case .blotting:
            DropText(title: "Blotting", detail: "The final pass")
        case .problem:
            DropText(title: "Far end silent", detail: "Nothing is arriving from the call", tone: .alert)
        }
    }

    /// A take: "Dictating · Slack · Chat" (the app in front and its mode) over its live words, or
    /// what it is doing when there are none.
    static func dictating(_ phase: CoreStore.DictationPhase, live: CoreStore.LiveDictation?) -> DropText {
        if live?.edit == true {
            return DropText(title: "Editing the selection", detail: phase == .transcribing ? "Rewriting" : "Say what to change")
        }
        let place = [live?.app, live?.mode].compactMap { $0 }.filter { !$0.isEmpty }
        let title = (["Dictating"] + place).joined(separator: " · ")
        if phase == .transcribing {
            return DropText(title: title, detail: "Transcribing")
        }
        if let words = live?.partial, !words.trimmingCharacters(in: .whitespaces).isEmpty {
            return DropText(title: title, detail: words, liveWords: true)
        }
        return DropText(title: title, detail: "Listening")
    }

    /// Where the wet words of `detail` start: the last `wetWords` words.
    static func wetStart(_ detail: String) -> String.Index {
        var index = detail.endIndex
        var words = 0
        var inWord = false
        for i in detail.indices.reversed() {
            let space = detail[i].isWhitespace
            if !space && !inWord {
                words += 1
                inWord = true
            } else if space && inWord {
                inWord = false
                if words == wetWords { return index }
            }
            if !space { index = i }
        }
        return detail.startIndex
    }
}

/// The orb in SwiftUI: behind the main window's content, or in the first run. Decorative: what it
/// shows is said by the Drop and the screens, so VoiceOver skips it.
struct OrbLayer: NSViewRepresentable {
    var state: InkState
    var palette: OrbPalette
    var placement: OrbPlacement
    /// "Always still" (Reduce Motion is the view's own check).
    var still: Bool
    /// Increase Contrast or Reduce Transparency: the orb dims behind the text.
    var dimmed: Bool
    /// The live levels it answers; the app's by default.
    var levels: @MainActor () -> InkLevels = ShellInk.liveLevels

    func makeNSView(context: Context) -> InkView {
        let view = InkView()
        view.setAccessibilityElement(false)
        return view
    }

    func updateNSView(_ view: InkView, context: Context) {
        view.levels = levels
        view.palette = palette
        view.placement = placement
        view.motionStill = still
        view.alphaValue = dimmed ? 0.45 : 1
        view.state = state
    }
}

/// The edge glow in SwiftUI, over the main window's content. It takes no clicks.
struct EdgeGlowLayer: NSViewRepresentable {
    var state: InkState
    var palette: OrbPalette
    /// Settings > Appearance: "Glow the window's edge".
    var on: Bool
    var still: Bool

    func makeNSView(context: Context) -> GlowEdgeView {
        let view = GlowEdgeView(style: Glow.edge)
        view.levels = ShellInk.liveLevels
        return view
    }

    func updateNSView(_ view: GlowEdgeView, context: Context) {
        view.palette = palette
        view.isOn = on
        view.motionStill = still
        view.state = state
    }
}
