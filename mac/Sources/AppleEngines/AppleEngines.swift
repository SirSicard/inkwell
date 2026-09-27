// Apple accelerators, registered into the core as engines over the C ABI (architecture rule 2):
// FluidAudio's Parakeet for live partials and Foundation Models for polish. A stub until S2.2.
import InkBridge

/// The engines this target registers. None yet.
public enum AppleEngines {
    /// Registers every Apple engine available on this Mac with `session`.
    public static func register(with session: InkSession) throws {}
}
