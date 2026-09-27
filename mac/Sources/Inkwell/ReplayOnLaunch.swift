// A meeting replayed from WAV files once the core is ready: INK_REPLAY_MEETING=<mic.wav>[,<far.wav>]
// runs the core's replay_meeting (the whole live chain and the final pass, from files instead of
// devices), so the Live screen can be seen and checked without a call (mac/SCREENS-B-CHECKLIST.md).
//
// Debug builds only: a replay writes a meeting nobody had into the library, so a release build
// (what build-mac.sh makes, and ships) has no such hook, and build-mac.sh refuses a release binary
// that names the variable.
#if DEBUG
import Foundation

enum ReplayOnLaunch {
    /// The replay_meeting command INK_REPLAY_MEETING asks for, if it asks: absolute paths only.
    static func command(from environment: [String: String]) -> [String: String]? {
        guard let raw = environment["INK_REPLAY_MEETING"], !raw.isEmpty else { return nil }
        let files = raw.split(separator: ",", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        guard let mic = files.first, mic.hasPrefix("/"), files.dropFirst().allSatisfy({ $0.hasPrefix("/") }) else {
            return nil
        }
        var command = ["cmd": "replay_meeting", "mic": mic, "title": "Replayed meeting"]
        if files.count == 2 {
            command["far"] = files[1]
        }
        return command
    }
}
#endif
