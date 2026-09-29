// When a screen's clock may run (Live's 1 s clock, the player's playhead): only while its screen is
// loaded, the window is on screen (not hidden to the tray or minimised), and something moves.
// Hidden, nothing ticks (architecture rule 9); shown again, the screen draws once, then ticks.
namespace Inkwell.Core.Screens;

public static class ScreenClock
{
    /// <summary>Whether the clock runs now.</summary>
    public static bool Runs(bool loaded, WindowPresence presence, bool moving)
    {
        ArgumentNullException.ThrowIfNull(presence);
        return loaded && presence.OnScreen && moving;
    }
}
