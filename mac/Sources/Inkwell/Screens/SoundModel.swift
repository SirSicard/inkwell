// Settings > Sound: the microphone Inkwell records with (one for dictation, meetings and the mic
// test alike: `audio.input`), what Automatic picks now and why, and the mic test with its level.
//
// The core holds the choice and resolves it against the devices connected now; this model only
// shows what it says. The list follows the devices as they come and go (`audio.devices_changed`,
// pushed by the core), so nothing here polls. The test's level arrives about ten times a second
// while a test runs, and not at all otherwise: the meter moves only then (architecture rule 9).
import Foundation
import InkBridge
import Observation

@MainActor
@Observable
final class SoundModel {
    /// What the core last said about the microphones.
    struct Devices: Equatable, Sendable {
        /// The connected microphones, the default first.
        let inputs: [AudioDevice]
        /// The choice: "auto", or a device's id.
        let input: String
        /// The chosen mic's name as remembered, shown even while it is not connected.
        let wantedName: String?
        /// What Automatic records now; nil when there is no microphone.
        let automatic: AudioInput?
        /// The mic a take, a meeting or a test would open now, and why.
        let using: AudioInput?
    }

    /// The mic test.
    enum Test: Equatable, Sendable {
        case idle
        /// Asked for; the mic is opening.
        case starting
        /// Running on `mic`: the level over the last tenth of a second (0 to 1).
        case running(mic: String, level: Double)
        /// Over: how, whether it heard more than a quiet room, and the core's words when it failed.
        case ended(AudioTestEnd, heard: Bool, message: String?)
    }

    /// A line of the picker.
    struct Choice: Identifiable, Equatable, Sendable {
        /// "auto", or the device's id.
        let id: String
        let title: String
    }

    private(set) var devices: Devices?
    private(set) var test: Test = .idle
    /// The devices could not be listed: gone with the next list.
    private var listProblem: String?
    /// A choice the core refused: said until the user picks again (the list read after the
    /// refusal shows the choice as it really is, and must not hide why it snapped back).
    private var choiceProblem: String?
    /// What is said under the picker.
    var problem: String? { choiceProblem ?? listProblem }
    /// Why the test could not start (a meeting records): said under the button.
    private(set) var testRefused: String?

    /// The command ids; every answer and failure carries one back.
    nonisolated static let devicesID = "sound.devices"
    nonisolated static let testID = "sound.test"
    nonisolated static let stopID = "sound.test_stop"
    nonisolated static let settingID = "setting:\(ShellSetting.audioInput.rawValue)"

    @ObservationIgnored private let send: SendCommand
    /// Settings shows Sound (between `load` and `disappeared`).
    @ObservationIgnored private var visible = false

    init(send: @escaping SendCommand) {
        self.send = send
    }

    /// Settings shows Sound: what the devices are now.
    func load() {
        visible = true
        send(.audioDevices(ref: Self.devicesID))
    }

    /// The user picked `id` ("auto" or a device's). Shown at once; the core checks the device is
    /// still connected, and its answer (or refusal) follows.
    func choose(_ id: String) {
        guard let devices, id != devices.input else { return }
        choiceProblem = nil
        // What records is unknown until the core answers (a stand-in's line would name the new mic
        // as missing meanwhile), and a test's result was about the mic before.
        self.devices = Devices(
            inputs: devices.inputs, input: id,
            wantedName: devices.inputs.first { $0.id == id }?.name,
            automatic: devices.automatic, using: nil)
        forgetTestResult()
        send(.settingSet(.audioInput, id))
    }

    /// Settings goes away: a running test stops (it would keep the mic open until its time is up),
    /// and a refused choice is old news when it comes back.
    func disappeared() {
        visible = false
        choiceProblem = nil
        if isTesting {
            send(.audioTestStop(ref: Self.stopID))
        }
    }

    /// A finished test's line is about the mic it ran on; another mic makes it stale, and so do
    /// other devices, unless it says why the test failed or stopped for a meeting (the devices
    /// change that follows a mic going mid-test must not hide why it stopped).
    private func forgetTestResult(keepingWhy: Bool = false) {
        guard case .ended(let how, _, _) = test else { return }
        if keepingWhy, how == .failed || how == .meeting { return }
        test = .idle
    }

    var isTesting: Bool {
        switch test {
        case .starting, .running: true
        case .idle, .ended: false
        }
    }

    /// Test, or Stop while one runs.
    func toggleTest() {
        if isTesting {
            send(.audioTestStop(ref: Self.stopID))
        } else {
            testRefused = nil
            test = .starting
            send(.audioTest(ref: Self.testID))
        }
    }

    /// Whether this model shows a failed command itself.
    func handles(_ failed: CommandFailed) -> Bool {
        [Self.devicesID, Self.testID, Self.stopID, Self.settingID].contains(failed.id ?? "")
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .coreReady, .coreStopped:
            // A test the core never ended (it stopped, or started again) is over.
            test = .idle
            testRefused = nil
            // A core started again while Settings shows: the devices as it sees them.
            if case .coreReady = event, visible { load() }
        case .audioDevices(let d):
            devices = Devices(inputs: d.inputs, input: d.input, wantedName: d.wanted?.name, automatic: d.automatic, using: d.using)
            listProblem = nil
        case .audioDevicesChanged(let d):
            devices = Devices(inputs: d.inputs, input: d.input, wantedName: d.wanted?.name, automatic: d.automatic, using: d.using)
            forgetTestResult(keepingWhy: true)
        // Only this screen's test (its ref): a late answer to one stopped earlier never ends the next.
        case .audioTestStarted(let started) where started.ref == Self.testID && isTesting:
            test = .running(mic: started.micName, level: 0)
            // Settings went while it was opening: its Stop reached the core first and found nothing.
            if !visible { send(.audioTestStop(ref: Self.stopID)) }
        case .audioTestLevel(let level) where level.ref == Self.testID:
            if case .running(let mic, _) = test {
                test = .running(mic: mic, level: min(max(level.level, 0), 1))
            }
        case .audioTested(let tested) where tested.ref == Self.testID && isTesting:
            test = .ended(tested.ended, heard: tested.heard, message: tested.message)
        case .commandFailed(let failed) where failed.id == Self.testID:
            test = .idle
            testRefused = failed.code == .meetingRecording
                ? "The test waits until the meeting ends."
                : "Couldn't start the test: \(failed.message)"
        case .commandFailed(let failed) where failed.id == Self.stopID:
            // No test was running: the screen already thinks so, or will when audio.tested lands.
            // (A Stop pressed while the test was still opening can reach the core first; the test
            // then runs to its end, and the button still offers Stop.)
            break
        case .commandFailed(let failed) where failed.id == Self.devicesID:
            listProblem = "Couldn't list the microphones: \(failed.message)"
        case .commandFailed(let failed) where failed.id == Self.settingID:
            choiceProblem = "Couldn't choose that microphone: \(failed.message)"
            // The choice as it really is.
            load()
        default:
            break
        }
    }

    // MARK: - What the section says

    /// The picker's lines: Automatic first (with what it records now), then each microphone with
    /// how it connects, and a chosen one that is not connected, so the choice still shows.
    var choices: [Choice] {
        guard let devices else { return [] }
        var out = [Choice(id: "auto", title: devices.automatic.map { "Automatic (\($0.name))" } ?? "Automatic")]
        out += devices.inputs.map { Choice(id: $0.id, title: Self.title($0.name, $0.transport)) }
        if devices.input != "auto", !devices.inputs.contains(where: { $0.id == devices.input }) {
            let name = devices.wantedName ?? "The microphone you chose"
            out.append(Choice(id: devices.input, title: "\(name) · not connected"))
        }
        return out
    }

    /// "AirPods Pro · Bluetooth": a device's name and how it connects, when that says something.
    static func title(_ name: String, _ transport: MicTransport) -> String {
        guard let word = transportWord(transport) else { return name }
        return "\(name) · \(word)"
    }

    static func transportWord(_ transport: MicTransport) -> String? {
        switch transport {
        case .builtIn: "Built-in"
        case .bluetooth: "Bluetooth"
        case .usb: "USB"
        case .virtual: "Virtual"
        case .other: nil
        }
    }

    /// The chosen mic is not connected, and Automatic stands in: the line that says so.
    var missingLine: String? {
        guard let devices, let using = devices.using, using.reason == .chosenMissing else { return nil }
        let name = devices.wantedName ?? "The microphone you chose"
        return "\(name) isn't connected. Inkwell is using \(using.name) until it is."
    }

    /// The caption under the picker: a choice that is missing, a Bluetooth mic's cost, or what
    /// Automatic does.
    var caption: String {
        guard let devices else { return "Reading the microphones…" }
        if let missingLine { return missingLine }
        if devices.inputs.isEmpty { return "No microphone is connected." }
        if devices.input != "auto" {
            if devices.using?.transport == .bluetooth {
                return "While Inkwell listens, a Bluetooth headset switches to call quality, for what you hear as well as your voice."
            }
            return "Dictation and meetings both use it. A meeting keeps its microphone unless it goes."
        }
        return "Automatic follows the Mac's input, but with Bluetooth headphones it uses the Mac's own microphone: a headset microphone carries only call-quality sound."
    }

    /// The level shown, 0 when no test runs.
    var level: Double {
        if case .running(_, let level) = test { return level }
        return 0
    }

    /// The line under the test button.
    var testLine: String {
        if let testRefused { return testRefused }
        switch test {
        case .idle:
            return "Speak for a few seconds to see the level. Nothing is kept."
        case .starting:
            return "Opening the microphone…"
        case .running(let mic, _):
            return "Listening with \(mic)…"
        case .ended(.meeting, _, _):
            return "Stopped: a meeting started recording."
        case .ended(.failed, _, let message):
            return "The test stopped: \(message ?? "the microphone failed")."
        case .ended(_, let heard, _):
            return heard
                ? "Inkwell heard you."
                : "Not hearing you? Check that this is the microphone you speak into, and that it isn't muted."
        }
    }

    /// Whether the test's line is a problem.
    var testLineIsProblem: Bool {
        if testRefused != nil { return true }
        switch test {
        case .ended(.failed, _, _): return true
        case .ended(.done, false, _), .ended(.stopped, false, _): return true
        default: return false
        }
    }
}
