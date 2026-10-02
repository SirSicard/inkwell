// The Glow orb and the edge glow: one Metal pipeline and a layer of gradient strokes, drawn only
// while something is live (architecture rule 9).
//
// | File | Holds |
// |---|---|
// | InkSimulation | the state, the levels' envelopes and the state weights, and the uniform block |
// | InkLevels | the core's audio bands as the live levels |
// | InkPipeline | the shader (Resources/ink.msl, generated from shaders/ink.wgsl) compiled at run time, off the main thread |
// | InkSchedule | when a view draws: a display link only while live, else one still frame at most |
// | InkClock | the one display link every live view shares |
// | InkView | the orb on screen (a transparent CAMetalLayer) |
// | GlowStyle | what the shell passes in: the orb's palette and placement, the edge's strokes, and the edge's frame |
// | GlowEdgeView | the window's edge glow (gradient strokes) |
// | InkSnapshot | one frame offscreen, at a fixed time: tests |
// | FrameCounter, GPUFrameTimes | what the shell budget reads: frames drawn, GPU time per frame |
import Metal

/// The ink's process-wide pieces.
public enum InkRenderer {
    /// Whether this Mac has a Metal device to draw on.
    public static var isSupported: Bool { MTLCreateSystemDefaultDevice() != nil }

    /// GPU time per frame, recorded only while a measurement asks for it.
    public static let gpuTimes = GPUFrameTimes()
}
