// What the ink shows and what moves it: the Mac renderer's InkState and InkVoice
// (mac/Sources/InkRenderer/InkSimulation.swift), for Windows.
using System.Runtime.Versioning;

[assembly: SupportedOSPlatform("windows10.0.26100.0")]

namespace Inkwell.Ink;

/// <summary>What the ink shows. The names are the design prototype's modes.</summary>
public enum InkState
{
    /// <summary>Nothing live: one still drop.</summary>
    Idle,
    /// <summary>A dictation: one wet drop that answers the voice.</summary>
    Dictating,
    /// <summary>A meeting: your drop and the far end's, one per stream.</summary>
    Meeting,
    /// <summary>The final pass: a blotting sheet lifts the ink.</summary>
    Blotting,
    /// <summary>The far end is silent: its drop fades to a ghost with a seal-red outline.</summary>
    Problem,
}

/// <summary>
/// Where the orb sits on its canvas: its centre as shares of the width and of the height (from the
/// top), and its unit as a share of the shorter side. The shell passes the tokens' placements
/// (GlowTokens.Orb); until then, the centre at the shorter side.
/// </summary>
public readonly record struct InkPlacement(double X, double Y, double Unit)
{
    /// <summary>Centred, one unit the shorter side.</summary>
    public static InkPlacement Centre => new(0.5, 0.5, 1);
}

/// <summary>InkState helpers.</summary>
public static class InkStates
{
    /// <summary>Every state, in declaration order.</summary>
    public static IReadOnlyList<InkState> All { get; } =
        [InkState.Idle, InkState.Dictating, InkState.Meeting, InkState.Blotting, InkState.Problem];

    /// <summary>Whether anything moves in this state (every state but idle).</summary>
    public static bool IsLive(this InkState state) => state != InkState.Idle;

    /// <summary>The prototype's mode name: idle, dictating, meeting, blotting, problem.</summary>
    public static string Name(this InkState state) => state switch
    {
        InkState.Idle => "idle",
        InkState.Dictating => "dictating",
        InkState.Meeting => "meeting",
        InkState.Blotting => "blotting",
        InkState.Problem => "problem",
        _ => throw new ArgumentOutOfRangeException(nameof(state)),
    };
}

/// <summary>The live levels the ink answers, each 0..1: your mic and the far end.</summary>
public readonly record struct InkLevels(double Near, double Far)
{
    /// <summary>No audio.</summary>
    public static InkLevels Silent => default;

    /// <summary>Reads 0 at and below this RMS level (dBFS): the room between words.</summary>
    public const double FloorDb = -55.0;

    /// <summary>Reads 1 at and above this RMS level (dBFS): only the loudest speech.</summary>
    public const double CeilingDb = -20.0;

    /// <summary>
    /// The level of one stream's bands (the core's RMS amplitude per band, linear full scale) on a
    /// decibel scale from <see cref="FloorDb"/> (0) to <see cref="CeilingDb"/> (1): the Mac's map,
    /// tuned on the core's measured fixture levels (InkLevels.swift). The ink's envelope follower
    /// smooths it, so this is a plain map; the same one serves your mic and the far end.
    /// </summary>
    public static double Level(float low, float mid, float high)
    {
        // The bands do not overlap, so their powers add.
        var power = ((double)low * low) + ((double)mid * mid) + ((double)high * high);
        if (!double.IsFinite(power) || power <= 0)
        {
            return 0;
        }
        var db = 10 * Math.Log10(power);
        return Math.Clamp((db - FloorDb) / (CeilingDb - FloorDb), 0, 1);
    }
}

/// <summary>
/// Where the voice that moves the ink comes from: the prototype's stand-in voice (reference
/// renders and tests only), or live levels.
/// </summary>
public readonly record struct InkVoice(bool IsSynthetic, double Near, double Far)
{
    /// <summary>The prototype's synthetic voice.</summary>
    public static InkVoice Synthetic => new(true, 0, 0);

    /// <summary>No voice at all.</summary>
    public static InkVoice Silent => default;

    /// <summary>Live levels, 0..1.</summary>
    public static InkVoice Levels(double near, double far) => new(false, near, far);
}
