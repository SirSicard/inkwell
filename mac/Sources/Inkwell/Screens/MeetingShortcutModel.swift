// The persisted global meeting toggle. Off by default on Mac; only core acknowledgements
// change the advertised key, so a failed save never replaces the previous shortcut in Settings.
import InkBridge
import Observation

@MainActor
@Observable
final class MeetingShortcutModel {
    private(set) var key = "off"
    private(set) var loaded = false
    private(set) var active = false
    private(set) var problem: String?
    @ObservationIgnored private let send: SendCommand
    static let settingID = "setting:\(ShellSetting.meetingsKey.rawValue)"

    init(send: @escaping SendCommand) { self.send = send }
    func load() { send(.settingGet(.meetingsKey)) }
    func setKey(_ key: String) {
        problem = nil
        send(.settingSet(.meetingsKey, key))
    }
    func handles(_ failed: CommandFailed) -> Bool { failed.id == Self.settingID }
    func apply(_ event: InkEvent) {
        switch event {
        case .meetingsShortcutState(let state):
            key = state.key
            loaded = true
            active = state.active
            problem = state.error
        case .commandFailed(let failed) where handles(failed):
            problem = "Couldn't read or save the meeting shortcut: \(failed.message). The previous key is unchanged."
        case .coreStopped:
            loaded = false
            active = false
        default: break
        }
    }
}
