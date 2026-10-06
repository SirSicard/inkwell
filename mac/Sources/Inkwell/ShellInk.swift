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
    /// The meeting commands: a Drop answer that failed is said in the Drop, and Stop and delete
    /// shows while it can delete.
    @ObservationIgnored let meetings: MeetingModel?
    /// The call policies: the Drop offers "Always for" an app not already Always, and says when
    /// setting one failed.
    @ObservationIgnored let calls: CallPolicyModel?
    /// A state held whatever the core says: the shell budget's live phase (INK_MEASURE=live) and
    /// the Drop's focus check (INK_DROP_DEMO). Nil in ordinary use.
    var held: InkState?

    init(
        store: CoreStore, permissions: PermissionsModel? = nil, meetings: MeetingModel? = nil,
        calls: CallPolicyModel? = nil
    ) {
        self.store = store
        self.permissions = permissions
        self.meetings = meetings
        self.calls = calls
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
        let record = store.meeting?.record
        return DropText.for(
            state, dictation: store.dictation, live: store.liveDictation, meeting: store.meeting,
            offer: store.offer, systemAudioOff: systemAudioOff,
            failure: meetings?.failure(on: .drop) ?? calls?.dropFailure,
            deletable: meetings?.canDiscard(record) ?? false,
            discarding: record != nil && meetings?.discarding == record,
            offerPolicy: store.offer.flatMap { calls?.policy(of: $0.app) })
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
        /// "Always for Zoom": the app's calls are recorded without asking from now on, and this
        /// one now (meetings.calls.set, then meeting.start).
        case always(app: String, name: String)
        /// "Never for Zoom": the app's calls are never offered or recorded (meetings.calls.set;
        /// the core withdraws the offer).
        case never(app: String, name: String)
        /// Stop a call its app's Always recorded (meeting.stop).
        case stop
        /// "Stop and delete", in that call's first minute (meeting.discard).
        case stopAndDelete
        /// Ask for system audio (as the Settings card does).
        case allowSystemAudio
        /// Bring the main window to Today, where the recommended speech models' download states
        /// its size and where the files come from. The Drop never starts a download itself: one
        /// starts only where those are shown.
        case showSpeechModels

        var title: String {
            switch self {
            case .record: "Record this call"
            case .dismiss: "Not this one"
            case .always(_, let name): "Always for \(DropText.named(name))"
            case .never(_, let name): "Never for \(DropText.named(name))"
            case .stop: "Stop"
            case .stopAndDelete: "Stop and delete"
            case .allowSystemAudio: "Allow system audio"
            case .showSpeechModels: "Download speech models\u{2026}"
            }
        }

        /// What VoiceOver says the button does, beyond its title.
        var hint: String? {
            switch self {
            case .always(_, let name):
                "Records this call, and from now on records \(DropText.named(name))'s calls without asking"
            case .never(_, let name):
                "Inkwell won't offer to record \(DropText.named(name))'s calls again"
            case .stopAndDelete: "Stops recording and deletes this recording, as if it had never been made"
            case .stop: "Stops recording; the final pass runs"
            default: nil
            }
        }
    }

    /// What the core calls an app whose name and identity show nothing.
    static var nameless: String { CallPolicyModel.nameless }

    /// An app's name in a button or a line: "this app" for the core's stand-in for none.
    static func named(_ name: String) -> String {
        name == nameless ? "this app" : name
    }

    /// The offer's line when an Always app's own sound can't be recorded alone
    /// (`meeting.detected`'s message is then the core's NOT_ALONE, word for word).
    static let notAloneMessage = "Inkwell can only record everything this computer plays for this app, so it asks first"

    /// Copy for each state, the tests read them.
    static let consentLine = "Recording keeps both sides on this Mac. Tell the others you are recording."
    /// The title names the app, so this line does not: a long name would push the reminder to
    /// tell the others past the Drop's last line.
    /// An Always app's start that failed (any other `meeting.detected` message).
    static let startFailedLine = "Inkwell couldn't start recording it by itself. Tell the others you are recording."
    static let notAloneLine = "Inkwell can't hear it alone: recording takes in everything this Mac plays. Tell the others you are recording."
    static func autoTitle(_ name: String?) -> String {
        "● Recording \(name.map(named) ?? "the call") automatically"
    }
    static func autoReminder(_ name: String?) -> String {
        "Always is on for \(name.map(named) ?? "this app"). Tell the others you are recording."
    }
    static let discardingTitle = "Stop and delete"
    static let discardingLine = "Deleting this recording"

    /// `text` with its first letter capitalised ("an app opened…" begins a line).
    static func sentence(_ text: String) -> String {
        text.prefix(1).uppercased() + text.dropFirst()
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
    ///
    /// The offer also sets the app's call policy: "Always for" (unless it is Always already:
    /// `offerPolicy`, or a `message` saying why an Always app is asked), and "Never for". A call
    /// its app's Always recorded says so, keeps the reminder to tell the others, and offers Stop,
    /// and Stop and delete while `deletable` (its first minute); `discarding` once that was pressed.
    static func `for`(
        _ state: InkState, dictation: CoreStore.DictationPhase, live: CoreStore.LiveDictation? = nil,
        meeting: CoreStore.LiveMeeting?, offer: CoreStore.Offer?, systemAudioOff: Bool,
        failure: String? = nil, deletable: Bool = false, discarding: Bool = false,
        offerPolicy: CallPolicy? = nil
    ) -> DropText {
        if discarding, state == .meeting || state == .blotting || state == .problem {
            return DropText(title: discardingTitle, detail: failure ?? discardingLine, tone: failure == nil ? .plain : .alert)
        }
        switch state {
        case .idle:
            guard let offer else { return DropText.for(state, dictation: dictation, live: live) }
            // An Always app asked instead: why, in the shell's words.
            let line = switch offer.message {
            case nil: consentLine
            case notAloneMessage?: notAloneLine
            // The platform's reason can run long and would cut the reminder: the core logged it.
            case _?: startFailedLine
            }
            let alreadyAlways = offer.message != nil || offerPolicy == .always
            var actions: [Action] = [.record(app: offer.app), .dismiss(app: offer.app)]
            if !alreadyAlways { actions.append(.always(app: offer.app, name: offer.appName)) }
            actions.append(.never(app: offer.app, name: offer.appName))
            return DropText(
                title: sentence("\(offer.appName) opened the microphone"),
                detail: failure ?? line,
                tone: failure == nil ? .plain : .alert,
                actions: actions)
        case .meeting:
            guard let meeting else { return DropText.for(state, dictation: dictation, live: live) }
            if meeting.auto {
                let latest = meeting.finals.last?.text.trimmingCharacters(in: .whitespacesAndNewlines)
                let reminder = autoReminder(meeting.appName)
                // The reminder holds the first minute, while Stop and delete is there; then the
                // latest line, as any meeting's.
                let detail = deletable ? reminder : latest.flatMap { $0.isEmpty ? nil : $0 } ?? reminder
                return DropText(
                    title: autoTitle(meeting.appName), detail: failure ?? detail,
                    tone: failure == nil ? .recording : .alert,
                    actions: deletable ? [.stop, .stopAndDelete] : [.stop])
            }
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
            // Only the probe knows the permission. Exact zeros are what a denied capture delivers,
            // but also what a call that went quiet delivers (an app holding its output open plays
            // them), so zeros alone are said as silence, and the permission is offered only when
            // the probe says it is off.
            let detail = if systemAudioOff {
                "System audio is off, so only your voice is being recorded."
            } else if far == .zeros {
                "Only silence is arriving from the call."
            } else {
                "Nothing is arriving from the call. Only your voice may be recorded."
            }
            // A call its app's Always recorded keeps its Stop (and Stop and delete) here too.
            var actions: [Action] = systemAudioOff ? [.allowSystemAudio] : []
            if meeting?.auto == true {
                actions += deletable ? [.stop, .stopAndDelete] : [.stop]
            }
            return DropText(title: title, detail: failure ?? detail, tone: .alert, actions: actions)
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
    /// Text sits over it (the main window's screens), not beside it (the first run's demo).
    var behindText = false
    /// The region it wanders in (the main window's, Glow.Orb.wander); nil keeps it at `placement`.
    var wanderBounds: OrbWander.Bounds?
    /// What it sits behind (the main window's route): a change at rest moves a wandering orb.
    var contentID: String?
    /// For what is drawn over it (the main window's milestone glow): holds it still and reads its
    /// spot.
    var hold: OrbHold?
    /// The live levels it answers; the app's by default.
    var levels: @MainActor () -> InkLevels = ShellInk.liveLevels

    /// How strongly a live orb shows behind text. Live, the orb takes the dot colours at full
    /// strength, and its bright centre sits behind every screen's text: at 30 % its brightest point
    /// still leaves the mode's text at 4.5:1 or more and its secondary text at 3:1 or more, with
    /// every preset in both modes (OrbBehindTextTests measures it; a colour of the user's own is
    /// not measured).
    nonisolated static let liveBehindText: CGFloat = 0.3

    /// How strongly an orb at rest shows behind text. At rest it is the mode's idle colour leaning
    /// well toward the preset (GlowColours.restTint), so each preset shows; dimmed this little, text
    /// keeps 4.5:1 and secondary text 3:1 or more over it, with every preset in both modes, wherever
    /// it wanders (OrbBehindTextTests). The pair is chosen together: see GlowColours.restTint.
    nonisolated static let restBehindText: CGFloat = 0.7

    /// How far blotting condenses an orb behind text (InkSimulation.blotDepth). The design's full
    /// blot ends in a small hard-edged drop of ink: in the Drop's pill it reads as that drop, but
    /// behind the main window it is a solid disc in the text's own colour (ivory at night),
    /// landing on whatever rule, divider or heading the window's layout puts at the orb's centre,
    /// for as long as the final pass runs. Behind text the blot stops on the way: the two orbs draw
    /// together, shrink and take the ink's colour, with a soft edge.
    nonisolated static let blotBehindText = 0.45

    nonisolated static func blotDepth(behindText: Bool) -> Double {
        behindText ? blotBehindText : 1
    }

    /// The orb's opacity for `state`.
    nonisolated static func opacity(state: InkState, behindText: Bool, dimmed: Bool) -> CGFloat {
        let rest: CGFloat = dimmed ? 0.45 : 1
        guard behindText else { return rest }
        return min(rest, state.isLive ? liveBehindText : restBehindText)
    }

    func makeNSView(context: Context) -> InkView {
        let view = InkView()
        view.setAccessibilityElement(false)
        view.alphaValue = Self.opacity(state: state, behindText: behindText, dimmed: dimmed)
        return view
    }

    func updateNSView(_ view: InkView, context: Context) {
        view.levels = levels
        view.palette = palette
        view.placement = placement
        view.motionStill = still
        view.blotDepth = Self.blotDepth(behindText: behindText)
        view.wanderBounds = wanderBounds
        hold?.attach(view)
        view.contentID = contentID
        let opacity = Self.opacity(state: state, behindText: behindText, dimmed: dimmed)
        // Compared with a margin, not exactly: if the opacity ever reads back rounded (a layer
        // keeps it as a Float), every update would start the fade again.
        if abs(view.alphaValue - opacity) > 0.001 {
            // Faded with the ink's own change of state (its colours ease in over about a second),
            // so the orb never jumps; still, it is set at once.
            if still || NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                view.alphaValue = opacity
            } else {
                NSAnimationContext.runAnimationGroup { context in
                    context.duration = 0.8
                    view.animator().alphaValue = opacity
                }
            }
        }
        view.state = state
    }
}

/// Holds the main window's orb still for something drawn over it (a milestone's glow) and says
/// where it is, so that thing sits on the orb wherever it has wandered, not at its home. OrbLayer
/// attaches the view. Holds are counted: a glow that ends late never lets go of the next one's.
@MainActor
final class OrbHold {
    private weak var view: InkView?
    private var holders = 0
    /// A hold has read the spot: it stays pinned (InkView.holdsSpot) until every hold is let go.
    private var spotRead = false

    /// How long a hold waits for the orb to arrive: a glide at rest, and a second's slack. The orb
    /// arrives on the display link's ticks, which can stop while the view still counts as on screen
    /// (the display asleep); and while its pipeline is still compiling it does not arrive at all.
    /// Unbounded, either would keep the orb held under a glow that never lights.
    static let arrivalLimit: Duration = .seconds(OrbWander.restGlide + 1)

    /// The orb's view; a new one takes the holds already made. A view replaced while a hold waits
    /// leaves that wait to its cancellation (OrbLayer keeps one view for the window's life).
    func attach(_ view: InkView) {
        guard self.view !== view else { return }
        self.view = view
        view.holdsStill = holders > 0
        view.holdsSpot = spotRead
    }

    /// Holds the orb and returns its centre (fractions of the view, y from the top) once it is on
    /// screen and has arrived (a glide under way, or the one it takes on coming on screen,
    /// finishes first), or wherever it is once `arrivalLimit` has passed; nil with no orb attached.
    /// Each hold needs its release, cancelled or not.
    func hold() async -> SIMD2<Double>? {
        holders += 1
        guard let view else { return nil }
        view.holdsStill = true
        await Self.settled(view, within: Self.arrivalLimit)
        guard !Task.isCancelled, let view = self.view else { return nil }
        spotRead = true
        view.holdsSpot = true
        return view.orbCentre
    }

    /// Waits for `view` to settle, at most `limit`, whichever ends first. Rule 9 (no polling
    /// timers) holds: this is a one-shot deadline, not a poll. It is set only inside a celebration
    /// that is already running and ends with the wait; it never wakes to look again, and nothing
    /// is scheduled while idle.
    private static func settled(_ view: InkView, within limit: Duration) async {
        let wait = Task { await view.settled() }
        // Past the limit it cancels the wait, which then ends at once; a wait that ended first
        // cancels it, and its sleep ends at once.
        let bound = Task {
            do { try await Task.sleep(for: limit) } catch { return }
            wait.cancel()
        }
        // A hold cancelled meanwhile stops waiting at once too.
        await withTaskCancellationHandler { await wait.value } onCancel: { wait.cancel() }
        bound.cancel()
    }

    /// Lets go of one hold; with none left the orb wanders again.
    func release() {
        guard holders > 0 else { return }
        holders -= 1
        guard holders == 0 else { return }
        spotRead = false
        view?.holdsSpot = false
        view?.holdsStill = false
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
