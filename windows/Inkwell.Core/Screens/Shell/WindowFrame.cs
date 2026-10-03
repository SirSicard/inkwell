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
}
