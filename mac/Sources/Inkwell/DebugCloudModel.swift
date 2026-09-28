// A stand-in cloud language model, for checking polish's consent step by hand
// (mac/DICTATION-CHECKLIST.md): INK_DEBUG_CLOUD_MODEL=<name> registers a model that says it is not
// on this Mac, under an id that sorts before Apple's, so polish would use it. The core then pauses
// polish (the user agreed to the on-device model, not to it) and the consent step names it as a
// cloud provider. It sends nothing anywhere: it answers with the words it was given. Local-only
// mode (on by default) refuses it even after consent, as it would a real cloud model.
//
// Debug builds only, like INK_REPLAY_MEETING: build-mac.sh refuses a release binary that names the
// variable.
#if DEBUG
import Foundation
import InkBridge

final class DebugCloudModel: InkLanguageModel {
    /// Sorts before "apple-foundation-models": the core picks the lowest id.
    let id = "0-debug-cloud"
    let licence = "none (a test stand-in)"
    let model: String
    let isLocal = false

    init(name: String) {
        model = name
    }

    /// The model INK_DEBUG_CLOUD_MODEL asks for, if it asks.
    static func fromEnvironment(_ environment: [String: String]) -> DebugCloudModel? {
        guard let name = environment["INK_DEBUG_CLOUD_MODEL"]?.trimmingCharacters(in: .whitespaces),
              !name.isEmpty
        else { return nil }
        return DebugCloudModel(name: name)
    }

    func generate(
        _ request: InkLlmRequest,
        cancellation: InkCancellation,
        completion: @escaping @Sendable (Result<String, InkEngineError>) -> Void
    ) {
        completion(.success(request.user))
    }
}
#endif
