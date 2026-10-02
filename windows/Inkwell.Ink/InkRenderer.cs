// The ink on Windows (Glow's orb): one Direct3D 11 pipeline, drawn only while something is live
// (architecture rule 9). The Mac's InkRenderer target, file for file where it can be:
//
// | File | Holds |
// |---|---|
// | InkSimulation | the prototype's state machine and droplet physics, and the uniform block |
// | InkPipeline | the shader (Shaders/ink.hlsl, generated from shaders/ink.wgsl) compiled at run time with FXC, off the UI thread (InkPipelineLoader) |
// | InkSchedule | when a surface draws: the clock only while live, else one still frame at most |
// | InkClock | the one frame clock every live surface shares: the compositor's clock, on a thread that exists only while something is live |
// | InkSurface | one ink on screen: state, schedule and simulation, drawing into a host (IInkTarget) |
// | DropWindow | the Drop: a raw Win32 window on DirectComposition that never takes focus |
// | GlowLook | the orb's colours and the Drop's pill, as the shell resolves them |
// | InkSnapshot | one frame offscreen, at a fixed time: tests and reference comparisons |
// | SystemMotion | Windows' Animation effects setting, the Reduce Motion of this shell |
using System.Diagnostics;

namespace Inkwell.Ink;

/// <summary>Frames the ink has presented, whichever surface drew them: the shell budget (I7) reads it; idle must draw none.</summary>
public static class InkFrames
{
    private static long count;

    /// <summary>Frames presented since launch.</summary>
    public static long Count => Interlocked.Read(ref count);

    /// <summary>Counts one presented frame. Any thread; never blocks or allocates.</summary>
    internal static void Tick() => Interlocked.Increment(ref count);
}

/// <summary>Where the ink's failures go, by name (never with anything the user said: the ink has no words).</summary>
public static class InkLog
{
    /// <summary>The sink; the app may replace it. Default: the debug trace.</summary>
    public static Action<string> Write { get; set; } = line => Trace.WriteLine($"Inkwell ink: {line}");
}
