// Today's card for the meeting being recorded, or blotted once it stopped: what it is and how long
// it has run, or how far the final pass has got, with Open and Stop.
//
// The clock in it moves once a second only while the meeting records and the window is on screen
// (the orb is drawing then anyway); blotting, it changes only when the core reports a step.
import InkBridge
import SwiftUI

struct LiveCard: View {
    let meeting: CoreStore.LiveMeeting
    @Environment(ScreenModels.self) private var screens
    @Environment(Router.self) private var router
    @Environment(WindowPresence.self) private var presence

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 3) {
                Text(Self.title(meeting))
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(Theme.text)
                    .lineLimit(1)
                meta
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                if let failure = screens.meetings.failure(on: .liveStop) {
                    Text(failure)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                }
            }
            Spacer(minLength: 0)
            Button("Open") { router.open(.live) }
                .buttonStyle(PaperButtonStyle())
                .accessibilityHint("Shows the meeting as it is written")
            if !meeting.stopping {
                Button("Stop") { screens.meetings.stop() }
                    .buttonStyle(PaperButtonStyle(prominent: true))
                    .accessibilityHint("Stops recording; the final pass then writes the record")
            }
        }
        .padding(.vertical, 16)
        .padding(.horizontal, 20)
        .paperCard()
        .accessibilityElement(children: .contain)
        .accessibilityLabel(meeting.stopping ? "Blotting" : "Recording")
    }

    @ViewBuilder private var meta: some View {
        if meeting.stopping {
            Text(Self.blottingLine(meeting))
        } else if let started = screens.live.startedAt, presence.onScreen {
            TimelineView(.periodic(from: started, by: 1)) { context in
                Text(Self.recordingLine(meeting, elapsedMs: screens.live.elapsedMs(at: context.date)))
                    .monospacedDigit()
            }
        } else {
            Text(Self.recordingLine(meeting, elapsedMs: screens.live.startedAt == nil ? nil : screens.live.elapsedMs()))
                .monospacedDigit()
        }
    }

    static func title(_ meeting: CoreStore.LiveMeeting) -> String {
        meeting.title ?? meeting.appName.map { "\($0) call" } ?? "Recording"
    }

    /// "Zoom · recording · 12:04".
    static func recordingLine(_ meeting: CoreStore.LiveMeeting, elapsedMs: Int64?) -> String {
        let app = meeting.title != nil ? meeting.appName : nil
        return ([app, "recording", elapsedMs.map { liveClock(ms: $0) }] as [String?])
            .compactMap { $0 }.joined(separator: " · ")
    }

    /// How far the final pass has got, from the steps the core has reported: "Blotting ·
    /// transcribing", then "Blotting · transcribed · speakers sorted · summarized". A step the core
    /// skips (one speaker on the far end, summaries off) is never shown as pending.
    static func blottingLine(_ meeting: CoreStore.LiveMeeting) -> String {
        // Both sides, or a later step (which comes after both).
        let transcribed = meeting.transcribed.count >= 2 || meeting.diarized || meeting.summarized
        var parts = ["Blotting", transcribed ? "transcribed" : "transcribing"]
        if meeting.diarized { parts.append("speakers sorted") }
        if meeting.summarized { parts.append("summarized") }
        return parts.joined(separator: " · ")
    }
}
