// The Stats screen's measures, kept apart from the view so the 720-epx window is tested headless:
// the room a card's content has at a window width, and the Records card's even columns (as the
// Mac's EvenColumns: as many as fit at their least width, each as wide as its share).
namespace Inkwell.Core.Screens;

public static class StatsLayout
{
    /// <summary>The navigation pane (MainWindow.xaml's OpenPaneLength).</summary>
    public const double NavigationPane = 220;

    /// <summary>The page's padding left and right (StatsScreen.xaml), and its widest, padding included.</summary>
    public const double PagePadding = 48;
    public const double PageWidth = 956;

    /// <summary>A card's padding left and right, and its border (App.xaml's InkCardStyle).</summary>
    public const double CardPadding = 22;
    public const double CardBorder = 1;

    /// <summary>A record's least width, and the room between records side by side and between rows.</summary>
    public const double RecordMinimum = 150;
    public const double RecordSpacing = 24;
    public const double RecordRowSpacing = 14;

    /// <summary>The narrowest window the app is laid out for.</summary>
    public const double MinimumWindow = 720;

    /// <summary>The width a Stats card's content has in a window <paramref name="window"/> wide.</summary>
    public static double CardContent(double window) =>
        Math.Min(window - NavigationPane, PageWidth) - 2 * PagePadding - 2 * CardPadding - 2 * CardBorder;

    /// <summary>How many even columns of at least <paramref name="minimum"/> fit <paramref name="available"/>, at most <paramref name="count"/>, and their width.</summary>
    public static (int Count, double Width) Columns(double available, double minimum, double spacing, int count)
    {
        if (double.IsInfinity(available) || double.IsNaN(available))
        {
            var n = Math.Max(1, Math.Min(count, 2));
            return (n, minimum);
        }
        var fit = Math.Max(1, (int)Math.Floor((available + spacing) / (minimum + spacing)));
        var columns = Math.Max(1, Math.Min(fit, count));
        return (columns, Math.Max(0, (available - spacing * (columns - 1)) / columns));
    }
}
