// The app's version, as the release stamps it: win-release.yml publishes with -p:InkVersion=X.Y.Z
// (from the tag), which Inkwell.csproj writes into the assembly as metadata. Any other build has
// none, and About says "development build", as the Mac does without a bundle version.
using System.Reflection;

namespace Inkwell;

internal static class AppVersion
{
    /// <summary>"1.0.0" in a release; null in any other build.</summary>
    public static string? Release { get; } =
        typeof(AppVersion).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
            .FirstOrDefault(a => a.Key == "InkVersion")?.Value;
}
