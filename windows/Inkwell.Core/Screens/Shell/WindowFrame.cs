// The main window's frame kept inside the display's work area (the screen less the taskbar): at
// launch, when it is shown from the tray and when it is restored. A window that comes back after a
// display was unplugged, or was made bigger than a smaller display, is moved and shrunk so all of
// it shows, as the Mac's WindowFrame.fitted does. Pixels, as Windows gives them.
namespace Inkwell.Core.Screens;

public readonly record struct WindowFrame(int X, int Y, int Width, int Height)
{
    /// <summary>
    /// This frame inside <paramref name="workArea"/>: no bigger than it, then moved the least that
    /// puts it all inside. A frame already inside comes back as it is.
    /// </summary>
    public WindowFrame Fitted(WindowFrame workArea)
    {
        var width = Math.Clamp(Width, 0, Math.Max(0, workArea.Width));
        var height = Math.Clamp(Height, 0, Math.Max(0, workArea.Height));
        var x = Math.Clamp(X, workArea.X, workArea.X + workArea.Width - width);
        var y = Math.Clamp(Y, workArea.Y, workArea.Y + workArea.Height - height);
        return new WindowFrame(x, y, width, height);
    }

    /// <summary>How many points across and down CoveredBy samples.</summary>
    private const int CoverSamples = 7;

    /// <summary>
    /// Whether <paramref name="above"/> (the windows over this one) hide all of it: every one of a
    /// grid of points across it, edges included, lies inside one of them. A grid, not exact
    /// geometry: a sliver left showing between two windows may count as covered, which is all the
    /// orb's wander needs (WindowCover).
    /// </summary>
    public bool CoveredBy(IReadOnlyList<WindowFrame> above)
    {
        ArgumentNullException.ThrowIfNull(above);
        if (Width <= 0 || Height <= 0 || above.Count == 0)
        {
            return false;
        }
        for (var i = 0; i < CoverSamples; i++)
        {
            for (var j = 0; j < CoverSamples; j++)
            {
                var px = X + (Width - 1) * i / (CoverSamples - 1);
                var py = Y + (Height - 1) * j / (CoverSamples - 1);
                if (!above.Any(a => px >= a.X && px < a.X + a.Width && py >= a.Y && py < a.Y + a.Height))
                {
                    return false;
                }
            }
        }
        return true;
    }
}
