// Stats: what the library says about the user's dictation and meetings, counted by the core on
// this Mac (stats.get), and the milestones it celebrates once each (milestones.check). Nothing is
// sent anywhere, compared with anyone, or drawn from what was said; the core's stats module says
// where each number comes from.
//
// Days are the user's: each question carries their time zone's UTC offsets over the last ten
// years (each from the moment it took effect, so a record made before a daylight-saving change
// keeps its day) and the weekday their calendar starts weeks on.
//
// Milestones are checked at launch (the core's first check notes what is already reached, so
// nothing old is celebrated) and whenever a dictation is saved or a meeting ends. A milestone
// reached waits here until the window is on screen, where ShellView shows its glow and note.
import Foundation
import InkBridge
import Observation

@MainActor
@Observable
final class StatsModel {
    /// Where the numbers are: asked and not answered, answered, or could not be counted.
    enum Load: Equatable, Sendable {
        case idle
        case loading
        case loaded
        case failed
    }

    /// A milestone reached, to celebrate once: a quiet glow on the orb and one line.
    struct Celebration: Equatable, Sendable {
        /// Increases with each, so a later one replaces the note and restarts its time.
        let serial: Int
        let id: String
        let note: String
    }

    /// The core's latest answer.
    private(set) var counted: StatsCounted?
    private(set) var loadState: Load = .idle
    /// The milestone to celebrate, until its note has been shown.
    private(set) var celebration: Celebration?
    /// Settings > Stats: milestones are celebrated (on unless turned off).
    private(set) var celebrate = true
    /// Settings > Stats: the typing speed time saved is measured against.
    private(set) var typingWpm = StatsModel.defaultTypingWpm
    /// A Stats setting could not be read or saved: Settings says so (it may not be what it shows).
    private(set) var settingsFailed = false

    /// The core's default typing speed, and the range it takes (ink-ffi's stats module).
    static let defaultTypingWpm = 40
    static let typingWpmRange = 10...200
    /// How far back the zone's offsets reach.
    nonisolated static let offsetYears = 10

    /// The calendar and clock the user's days are counted in.
    @ObservationIgnored var calendar = Calendar.autoupdatingCurrent
    @ObservationIgnored var now: () -> Date = Date.init

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var sequence = 0
    @ObservationIgnored private var latestGet: String?
    @ObservationIgnored private var latestCheck: String?
    /// Whether the Stats screen shows: it then counts again when something is saved.
    @ObservationIgnored private var shown = false
    @ObservationIgnored private var celebrationSerial = 0

    /// The ids of this model's setting commands (CoreCommand.json gives each setting command one).
    static let settingIDs: Set<String> = [
        "setting:\(ShellSetting.statsCelebrate.rawValue)", "setting:\(ShellSetting.statsTypingWpm.rawValue)",
    ]

    init(send: @escaping SendCommand) {
        self.send = send
    }

    // MARK: Questions

    private func ref(_ name: String) -> String {
        sequence += 1
        return "\(name)-\(sequence)"
    }

    /// The calendar fields both questions carry.
    private var calendarFields: (offsets: [CoreCommand.UTCOffset], weekStart: Int) {
        (Self.utcOffsets(timeZone: calendar.timeZone, now: now()), Self.isoWeekStart(calendar))
    }

    /// Counts again.
    func load() {
        if loadState != .loaded { loadState = .loading }
        let ref = ref("stats")
        latestGet = ref
        let fields = calendarFields
        send(.statsGet(utcOffsets: fields.offsets, weekStart: fields.weekStart, ref: ref))
    }

    /// Asks what is newly reached.
    func checkMilestones() {
        let ref = ref("milestones")
        latestCheck = ref
        let fields = calendarFields
        send(.milestonesCheck(utcOffsets: fields.offsets, weekStart: fields.weekStart, ref: ref))
    }

    func screenAppeared() {
        shown = true
        load()
    }

    func screenDisappeared() {
        shown = false
    }

    func setCelebrate(_ on: Bool) {
        celebrate = on
        send(.settingSet(.statsCelebrate, on ? "on" : "off"))
    }

    /// Sets the typing speed, within what the core takes.
    func setTypingWpm(_ wpm: Int) {
        let clamped = min(max(wpm, Self.typingWpmRange.lowerBound), Self.typingWpmRange.upperBound)
        typingWpm = clamped
        send(.settingSet(.statsTypingWpm, String(clamped)))
    }

    /// The note has been shown for its time (or dismissed): it goes, unless a later one replaced it.
    func dismissCelebration(_ serial: Int) {
        if celebration?.serial == serial { celebration = nil }
    }

    /// Whether this model shows `failed` itself.
    func handles(_ failed: CommandFailed) -> Bool {
        switch failed.command {
        case "stats.get": failed.id?.hasPrefix("stats-") == true
        case "setting.get", "setting.set": Self.settingIDs.contains(failed.id ?? "")
        default: false
        }
    }

    // MARK: Answers

    func apply(_ event: InkEvent) {
        switch event {
        case .coreReady:
            send(.settingGet(.statsCelebrate))
            send(.settingGet(.statsTypingWpm))
            checkMilestones()
        case .dictationInserted(let inserted) where inserted.record != nil:
            somethingSaved()
        case .meetingFinished:
            somethingSaved()
        case .importFinished:
            if shown { load() }
        case .statsCounted(let answer) where answer.ref != nil && answer.ref == latestGet:
            counted = answer
            loadState = .loaded
        case .milestonesReached(let answer) where answer.ref != nil && answer.ref == latestCheck:
            // Usually none; several at once only after a long gap. One line says the biggest.
            if let last = answer.milestones.last {
                celebrationSerial += 1
                celebration = Celebration(
                    serial: celebrationSerial, id: last.id,
                    note: StatsFormat.milestoneNote(kind: last.kind, threshold: last.threshold, locale: calendar.locale ?? .current))
            }
        case .settingValue(let value) where value.key == ShellSetting.statsCelebrate.rawValue:
            settingsFailed = false
            celebrate = value.value != "off"
        case .settingValue(let value) where value.key == ShellSetting.statsTypingWpm.rawValue:
            // Read as the core reads it: anything it would not take is the default.
            let wpm = value.value.flatMap(Int.init).flatMap { Self.typingWpmRange.contains($0) ? $0 : nil }
            let changed = (wpm ?? Self.defaultTypingWpm) != typingWpm
            typingWpm = wpm ?? Self.defaultTypingWpm
            // Time saved is counted against it.
            if loadState == .loaded, changed || Int(counted?.typingWpm ?? 0) != typingWpm { load() }
        case .commandFailed(let failed) where failed.command == "stats.get" && failed.id != nil && failed.id == latestGet:
            loadState = .failed
        case .commandFailed(let failed) where Self.settingIDs.contains(failed.id ?? ""):
            settingsFailed = true
        default:
            break
        }
    }

    private func somethingSaved() {
        checkMilestones()
        if shown { load() }
    }

    // MARK: The user's calendar

    /// `timeZone`'s UTC offset over the last ten years: the offset then, and each change since,
    /// oldest first (what stats.get's `utc_offsets` takes).
    nonisolated static func utcOffsets(timeZone: TimeZone, now: Date) -> [CoreCommand.UTCOffset] {
        let start = Calendar(identifier: .gregorian).date(byAdding: .year, value: -offsetYears, to: now) ?? now
        var offsets = [CoreCommand.UTCOffset(
            fromUnixMs: Int64((start.timeIntervalSince1970 * 1000).rounded()),
            minutes: timeZone.secondsFromGMT(for: start) / 60)]
        var at = start
        // Two changes a year at most in any zone: the cap only guards a zone database gone wrong.
        while offsets.count < 300, let next = timeZone.nextDaylightSavingTimeTransition(after: at), next <= now {
            let minutes = timeZone.secondsFromGMT(for: next) / 60
            if minutes != offsets.last?.minutes {
                offsets.append(.init(fromUnixMs: Int64((next.timeIntervalSince1970 * 1000).rounded()), minutes: minutes))
            }
            at = next
        }
        return offsets
    }

    /// The ISO weekday (1 Monday to 7 Sunday) `calendar` starts weeks on; Foundation counts from
    /// Sunday as 1.
    nonisolated static func isoWeekStart(_ calendar: Calendar) -> Int {
        (calendar.firstWeekday + 5) % 7 + 1
    }
}
