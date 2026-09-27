// llama.cpp's Metal backend keeps a loaded model's memory resident with a heartbeat: a background
// loop that wakes every 5 ms for as long as the model is loaded (ggml_metal_rsets_init), and asks
// for residency again on each wake for 3 minutes after the model's last use. With the dictation
// model warm, that loop alone cost 0.23-0.29 % of a core at idle, over the shell's idle budget
// (invariant I7: under 0.1 %). With residency sets off, idle measured 0.017 %.
//
// The cost of turning them off is the first decode after a quiet spell. Measured once on a 6 s clip:
// 487 ms after 240 s idle with residency off, against 377 ms with it on (by then residency had
// lapsed too, 3 minutes after the last use). Warm decodes were the same, about 305 ms either way.
// Warming the dictation engine when the key goes down is meant to hide it; until that is built and
// measured, the first dictation after a quiet spell pays it.
//
// Called first in main(): ggml reads the variable when the core loads the model, much later, and
// setenv is unsafe beside a getenv on another thread, so it runs before anything else starts.
import Foundation

enum MetalResidency {
    /// The variable llama.cpp's Metal backend reads: set (to anything), residency sets are off.
    static let ggmlVariable = "GGML_METAL_NO_RESIDENCY"
    /// Set to "1" to keep llama.cpp's residency heartbeat, for a benchmark that compares the two.
    static let keepVariable = "INK_METAL_RESIDENCY"

    /// Turns llama.cpp's Metal residency sets off, unless `INK_METAL_RESIDENCY=1` asks to keep them.
    /// A value already set for the ggml variable is left alone. Returns whether residency is off.
    @discardableResult
    static func configure(environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        if environment[keepVariable] == "1" { return environment[ggmlVariable] != nil }
        if environment[ggmlVariable] == nil {
            setenv(ggmlVariable, "1", 0)
        }
        return true
    }
}
