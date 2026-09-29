// One pipeline for the GPU tests, made once. WARP by default: CI's runner and an SSH session have
// no desktop and maybe no GPU, and WARP renders the same everywhere. INK_TEST_ADAPTER=hardware
// draws on the GPU instead (a person at the PC, or a GPU that works without a session).
using Inkwell.Ink;

// The ink is Windows-only, as are these tests (the library's own floor).
[assembly: System.Runtime.Versioning.SupportedOSPlatform("windows10.0.26100.0")]

namespace Inkwell.Ink.Tests;

internal static class TestPipeline
{
    private static readonly Lazy<InkPipeline> Shared = new(() => new InkPipeline(Adapter));

    public static InkAdapter Adapter =>
        Environment.GetEnvironmentVariable("INK_TEST_ADAPTER") == "hardware" ? InkAdapter.Hardware : InkAdapter.Warp;

    /// <summary>The pipeline. Direct3D's immediate context is single-threaded: tests that draw take <see cref="Lock"/>.</summary>
    public static InkPipeline Get() => Shared.Value;

    public static readonly Lock Lock = new();
}
