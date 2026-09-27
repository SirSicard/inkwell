// The ink: one Metal pipeline, drawn only while something is live (architecture rule 9). A stub
// until S2.4.
import Metal

/// The renderer. Nothing is drawn yet.
public enum InkRenderer {
    /// Whether this Mac has a Metal device to draw on.
    public static var isSupported: Bool { MTLCreateSystemDefaultDevice() != nil }
}
