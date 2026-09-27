// The ink the shell shows: one state for every surface that draws it (the Drop, the window's rail,
// Today's ink zone), read from the core's events through the store. The core pushes, the ink
// follows: nothing here polls.
//
//   dictation listening or transcribing   dictating
//   a meeting recording                   meeting, or problem while the far end delivers nothing
//   a meeting stopped, final pass running blotting
//   nothing live                          idle
import InkBridge
import InkRenderer
import Observation
import SwiftUI

@MainActor
@Observable
final class ShellInk {
    @ObservationIgnored let store: CoreStore
    /// A state held whatever the core says: the shell budget's live phase (INK_MEASURE=live) and
    /// the Drop's focus check (INK_DROP_DEMO). Nil in ordinary use.
    var held: InkState?

    init(store: CoreStore) {
        self.store = store
    }

    /// What the ink shows now.
    var state: InkState {
        held ?? Self.state(meeting: store.meeting, dictation: store.dictation)
    }

    /// What the Drop says beside it.
    var dropText: DropText {
        DropText.for(state, dictation: store.dictation, live: store.liveDictation)
    }

    /// The state for what is live. A meeting outranks a dictation. Only the far end sets the
    /// problem state, because the problem ink shows the far end's drop gone dead; a silent mic
    /// shows as your own drop lying still.
    nonisolated static func state(meeting: CoreStore.LiveMeeting?, dictation: CoreStore.DictationPhase) -> InkState {
        if let meeting {
            if meeting.stopping { return .blotting }
            if let far = meeting.sides[.far], far != .ok { return .problem }
            return .meeting
        }
        return dictation == .idle ? .idle : .dictating
    }

    /// The live levels, read by each ink at vsync. The core's bands are your mic's
    /// (ink_bands_read); it publishes none for the far end yet, so the far end's drop keeps its
    /// resting size.
    static func liveLevels() -> InkLevels {
        let bands = InkSession.bands()
        return InkLevels(near: InkLevels.level(low: bands.low, mid: bands.mid, high: bands.high), far: 0)
    }
}

/// The Drop's two lines. While a dictation is held the second holds its live words.
struct DropText: Equatable, Sendable {
    /// How the Drop colours the title and its border.
    enum Tone: Equatable, Sendable {
        case plain
        /// A recording: the title in seal red, as a recording light.
        case recording
        /// Something needs the user: the title and the border in seal red.
        case alert
    }

    var title: String
    var detail: String
    var tone = Tone.plain
    /// The detail is the live words of a take being held: its end matters (the head is cut, not
    /// the tail), and its newest words are still wet.
    var liveWords = false

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

/// An ink zone in SwiftUI: the rail beside the content, or Today's zone with the wordmark.
struct InkZone: NSViewRepresentable {
    var state: InkState
    var showsWordmark: Bool

    func makeNSView(context: Context) -> InkView {
        let view = InkView()
        view.levels = ShellInk.liveLevels
        return view
    }

    func updateNSView(_ view: InkView, context: Context) {
        view.state = state
        view.showsWordmark = showsWordmark
    }
}
