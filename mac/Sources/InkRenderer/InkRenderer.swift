// The ink: one Metal pipeline, drawn only while something is live (architecture rule 9).
//
// | File | Holds |
// |---|---|
// | InkSimulation | the prototype's state machine and droplet physics, and the uniform block |
// | InkLevels | the core's audio bands as the ink's live levels |
// | InkPipeline | the shader (Resources/ink.msl, generated from shaders/ink.wgsl) compiled at run time, off the main thread |
// | InkSchedule | when a view draws: a display link only while live, else one still frame at most |
// | InkView | the ink on screen (CAMetalLayer) |
// | Wordmark | INKWELL, rasterised with CoreText, knocked out of the ink |
// | InkSnapshot | one frame offscreen, at a fixed time: tests and reference comparisons |
// | FrameCounter, GPUFrameTimes | what the shell budget reads: frames drawn, GPU time per frame |
import Metal

/// The ink's process-wide pieces.
public enum InkRenderer {
    /// Whether this Mac has a Metal device to draw on.
    public static var isSupported: Bool { MTLCreateSystemDefaultDevice() != nil }

    /// GPU time per frame, recorded only while a measurement asks for it.
    public static let gpuTimes = GPUFrameTimes()
}
