// Windows' "Animation effects" (Settings > Accessibility > Visual effects), the Reduce Motion of
// this shell: off, every ink shows its state as one still frame and nothing moves. Read with
// SystemParametersInfo(SPI_GETCLIENTAREAANIMATION); a change is heard as WM_SETTINGCHANGE by the
// Drop's window, which exists from launch to quit (hidden or not) and passes it on to every ink.
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.SPI;

namespace Inkwell.Ink;

/// <summary>The animation setting.</summary>
public static class SystemMotion
{
    private static bool? last;

    /// <summary>Whether Windows allows animation. On a failed read, yes (and the failure is logged).</summary>
    public static unsafe bool AnimationsEnabled
    {
        get
        {
            BOOL enabled = true;
            if (!Windows.SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &enabled, 0))
            {
                InkLog.Write($"couldn't read the Animation effects setting (error {Windows.GetLastError()}); animating");
                return true;
            }
            return enabled;
        }
    }

    /// <summary>The setting changed. Raised on the UI thread.</summary>
    public static event Action? Changed;

    /// <summary>A WM_SETTINGCHANGE arrived: re-read, and tell every ink when the answer changed. UI thread.</summary>
    internal static void SettingChanged()
    {
        var now = AnimationsEnabled;
        if (last != now)
        {
            last = now;
            Changed?.Invoke();
        }
    }
}
