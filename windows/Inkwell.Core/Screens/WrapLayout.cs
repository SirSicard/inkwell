// Where items go when they wrap: left to right, a new row when the next one would not fit, each
// item whole (a row breaks between items, never inside one). The view's WrapPanel measures its
// children and places them here; kept apart from it so the rule is tested headless.
namespace Inkwell.Core.Screens;

public static class WrapLayout
{
    /// <summary>
    /// Each item's place, and the size of all of them, for rows <paramref name="available"/> wide.
    /// An item with no size (collapsed) takes no place and no spacing; one wider than a row gets
    /// a row of its own.
    /// </summary>
    public static (IReadOnlyList<(double X, double Y)> Places, double Width, double Height) Place(
        IReadOnlyList<(double Width, double Height)> items, double available, double spacing, double rowSpacing)
    {
        ArgumentNullException.ThrowIfNull(items);
        var places = new List<(double X, double Y)>(items.Count);
        double x = 0, y = 0, rowHeight = 0, width = 0;
        var rowHasItems = false;
        foreach (var (w, h) in items)
        {
            if (w <= 0 && h <= 0)
            {
                places.Add((x, y));
                continue;
            }
            if (rowHasItems && x + spacing + w > available)
            {
                y += rowHeight + rowSpacing;
                x = 0;
                rowHeight = 0;
                rowHasItems = false;
            }
            if (rowHasItems)
            {
                x += spacing;
            }
            places.Add((x, y));
            x += w;
            rowHeight = Math.Max(rowHeight, h);
            width = Math.Max(width, x);
            rowHasItems = true;
        }
        return (places, width, rowHasItems || y > 0 ? y + rowHeight : 0);
    }
}
