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
// nothing old is celebrated) and whenever a dictation is saved or a meeting ends. The core reports
// each milestone once ever, so every check's answer counts, not only the newest: two checks in
// flight would otherwise lose one. A milestone reached waits here until the window is on screen,
// where ShellView shows its glow and note, each once.
//
// The days are the user's current zone's: a record made while travelling is placed by the zone
// the Mac is in now, as the store keeps no zone per record.
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

    /// The question being answered while the numbers load, for the screen's time limit.
    private(set) var pendingLoad: String?

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var sequence = 0
    @ObservationIgnored private var latestGet: String?
    /// The Stats screen shows, and its window is on screen: it then counts again when something is
    /// saved. Off screen it waits, and counts when it is back.
    @ObservationIgnored private var screenShown = false
    @ObservationIgnored private var windowOnScreen = true
    @ObservationIgnored private var celebrationSerial = 0
    /// The celebration whose glow, and whose line read aloud, have played: each once, even if the
    /// window leaves the screen and comes back while the line still waits.
    @ObservationIgnored private var glowedSerial: Int?
    @ObservationIgnored private var announcedSerial: Int?
    /// Each setting's own writes not yet echoed by the core: an echo of an earlier write is not
    /// taken over a later one (quick Stepper clicks would step back).
    @ObservationIgnored private var unechoed: [ShellSetting: Int] = [:]

    /// How long the screen waits for its numbers before it says it couldn't count them.
    static let loadLimit: Duration = .seconds(20)

    /// The milestones, smallest first within each kind and words before streaks: the core's
    /// fixed order. When two arrive at once, the line says the later one.
    static let milestoneOrder = [
        "words_1000", "words_10000", "words_50000", "words_100000", "streak_7", "streak_30", "streak_100",
    ]

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
        pendingLoad = ref
        let fields = calendarFields
        send(.statsGet(utcOffsets: fields.offsets, weekStart: fields.weekStart, ref: ref))
    }

    /// Asks what is newly reached.
    func checkMilestones() {
        let ref = ref("milestones")
        let fields = calendarFields
        send(.milestonesCheck(utcOffsets: fields.offsets, weekStart: fields.weekStart, ref: ref))
    }

    private var visible: Bool { screenShown && windowOnScreen }

    func screenAppeared() {
        screenShown = true
        load()
    }

    func screenDisappeared() {
        screenShown = false
    }

    /// The window came on screen or left it. Back on screen, a showing Stats counts again.
    func windowPresence(onScreen: Bool) {
        let was = visible
        windowOnScreen = onScreen
        if visible && !was { load() }
    }

    /// The numbers did not come within the time limit: said, rather than a spinner for ever. Only
    /// while nothing is shown yet: a refresh that hangs leaves the last numbers up.
    func loadTimedOut(_ ref: String) {
        guard ref == latestGet, loadState == .loading else { return }
        loadState = .failed
        pendingLoad = nil
    }

    func setCelebrate(_ on: Bool) {
        celebrate = on
        write(.statsCelebrate, on ? "on" : "off")
    }

    /// Sets the typing speed, within what the core takes.
    func setTypingWpm(_ wpm: Int) {
        let clamped = min(max(wpm, Self.typingWpmRange.lowerBound), Self.typingWpmRange.upperBound)
        typingWpm = clamped
        write(.statsTypingWpm, String(clamped))
    }

    private func write(_ key: ShellSetting, _ value: String) {
        unechoed[key, default: 0] += 1
        send(.settingSet(key, value))
    }

    /// Whether `key`'s `setting.value` is an echo of one of this model's own writes with a later
    /// one still to come: it is then not taken. The last echo is (the core's word on it).
    private func earlierEcho(_ key: ShellSetting) -> Bool {
        guard let pending = unechoed[key], pending > 0 else { return false }
        unechoed[key] = pending - 1
        return pending > 1
    }

    /// The glow for `serial` may play: true once per celebration.
    func beginGlow(_ serial: Int) -> Bool {
        guard glowedSerial != serial else { return false }
        glowedSerial = serial
        return true
    }

    /// The line for `serial` may be read aloud: true once per celebration.
    func beginAnnouncement(_ serial: Int) -> Bool {
        guard announcedSerial != serial else { return false }
        announcedSerial = serial
        return true
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
            // A core that started again answers none of the old one's writes.
            unechoed = [:]
            send(.settingGet(.statsCelebrate))
            send(.settingGet(.statsTypingWpm))
            checkMilestones()
        case .dictationInserted(let inserted) where inserted.record != nil:
            somethingSaved()
        case .meetingFinished:
            somethingSaved()
        case .importFinished:
            if visible { load() }
        case .statsCounted(let answer) where answer.ref != nil && answer.ref == latestGet:
            counted = answer
            loadState = .loaded
            pendingLoad = nil
        case .milestonesReached(let answer) where answer.ref?.hasPrefix("milestones-") == true:
            // Every check's answer: each milestone is reported once ever. Usually none; several
            // only after a long gap, or from checks that overlapped. One line says the biggest.
            let rank = { (id: String) in Self.milestoneOrder.firstIndex(of: id) ?? -1 }
            guard let biggest = answer.milestones.max(by: { rank($0.id) < rank($1.id) }) else { break }
            if let pending = celebration, rank(pending.id) >= rank(biggest.id) { break }
            celebrationSerial += 1
            celebration = Celebration(
                serial: celebrationSerial, id: biggest.id,
                note: StatsFormat.milestoneNote(
                    kind: biggest.kind, threshold: biggest.threshold, locale: calendar.locale ?? .current))
        case .settingValue(let value) where value.key == ShellSetting.statsCelebrate.rawValue:
            if earlierEcho(.statsCelebrate) { break }
            settingsFailed = false
            celebrate = value.value != "off"
        case .settingValue(let value) where value.key == ShellSetting.statsTypingWpm.rawValue:
            if earlierEcho(.statsTypingWpm) { break }
            settingsFailed = false
            // Read as the core reads it: anything it would not take is the default.
            let wpm = value.value.flatMap(Int.init).flatMap { Self.typingWpmRange.contains($0) ? $0 : nil }
            typingWpm = wpm ?? Self.defaultTypingWpm
            // Time saved is counted against it; a hidden Stats counts when it shows.
            if visible, let counted, Int(counted.typingWpm) != typingWpm { load() }
        case .commandFailed(let failed) where failed.command == "stats.get" && failed.id != nil && failed.id == latestGet:
            loadState = .failed
            pendingLoad = nil
        case .commandFailed(let failed) where Self.settingIDs.contains(failed.id ?? ""):
            if failed.command == "setting.set", let key = Self.settingKey(failed.id) {
                _ = earlierEcho(key)
            }
            settingsFailed = true
        default:
            break
        }
    }

    private static func settingKey(_ id: String?) -> ShellSetting? {
        guard let id, id.hasPrefix("setting:") else { return nil }
        return ShellSetting(rawValue: String(id.dropFirst("setting:".count)))
    }

    private func somethingSaved() {
        checkMilestones()
        if visible { load() }
    }

    // MARK: The user's calendar

    /// `timeZone`'s UTC offset over the last ten years: the offset then, and each change since,
    /// oldest first (what stats.get's `utc_offsets` takes). Every change, not only daylight
    /// saving's: a zone that moved its standard time (as North Korea's did in 2018) is sampled
    /// weekly, and each change found is narrowed to the second.
    nonisolated static func utcOffsets(timeZone: TimeZone, now: Date) -> [CoreCommand.UTCOffset] {
        let start = Calendar(identifier: .gregorian).date(byAdding: .year, value: -offsetYears, to: now) ?? now
        let ms = { (date: Date) in Int64((date.timeIntervalSince1970 * 1000).rounded()) }
        let minutes = { (date: Date) in timeZone.secondsFromGMT(for: date) / 60 }
        var offsets = [CoreCommand.UTCOffset(fromUnixMs: ms(start), minutes: minutes(start))]
        let week: TimeInterval = 7 * 86_400
        var from = start
        // Two changes a year at most in any zone, and one week cannot hold two: the cap only
        // guards a zone database gone wrong.
        while from < now, offsets.count < 300 {
            let to = min(from.addingTimeInterval(week), now)
            if minutes(to) != minutes(from) {
                // The first second with the new offset.
                var (before, after) = (from, to)
                while after.timeIntervalSince(before) > 1 {
                    let middle = before.addingTimeInterval(after.timeIntervalSince(before) / 2)
                    if minutes(middle) == minutes(from) { before = middle } else { after = middle }
                }
                // Zones change on a whole second, the one after `before`.
                let change = Date(timeIntervalSince1970: before.timeIntervalSince1970.rounded(.down) + 1)
                if minutes(to) != offsets.last?.minutes {
                    offsets.append(.init(fromUnixMs: ms(min(change, after)), minutes: minutes(to)))
                }
            }
            from = to
        }
        return offsets
    }

    /// The ISO weekday (1 Monday to 7 Sunday) `calendar` starts weeks on; Foundation counts from
    /// Sunday as 1.
    nonisolated static func isoWeekStart(_ calendar: Calendar) -> Int {
        (calendar.firstWeekday + 5) % 7 + 1
    }
}
