// Children in even columns, as many as fit at MinColumnWidth each (StatsLayout.Columns), left to
// right in rows; each child as wide as its column, so its text wraps there; each row as tall as
// its tallest. The Records card's grid, as the Mac's EvenColumns.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace Inkwell.Screens;

public sealed partial class EvenColumnsPanel : Panel
{
    public double MinColumnWidth { get; set; } = StatsLayout.RecordMinimum;

    /// <summary>Between columns.</summary>
    public double Spacing { get; set; } = StatsLayout.RecordSpacing;

    /// <summary>Between rows.</summary>
    public double RowSpacing { get; set; } = StatsLayout.RecordRowSpacing;

    /// <summary>The columns the children were measured in, which arrange keeps when its width allows.</summary>
    private (int Count, double Width) measured = (1, 0);

    protected override Size MeasureOverride(Size availableSize)
    {
        var (count, width) = StatsLayout.Columns(availableSize.Width, MinColumnWidth, Spacing, Children.Count);
        measured = (count, width);
        foreach (var child in Children)
        {
            child.Measure(new Size(width, double.PositiveInfinity));
        }
        var rows = Rows(count);
        var height = rows.Sum() + RowSpacing * Math.Max(0, rows.Count - 1);
        var total = double.IsInfinity(availableSize.Width) ? width * count + Spacing * (count - 1) : availableSize.Width;
        return new Size(Math.Max(0, total), height);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        // The columns the children were measured in (their heights are for those widths), unless
        // the final width cannot hold them (measured without a width): then the final width's.
        var (count, width) = measured;
        if (count * width + (count - 1) * Spacing > finalSize.Width + 0.5)
        {
            (count, width) = StatsLayout.Columns(finalSize.Width, MinColumnWidth, Spacing, Children.Count);
        }
        var rows = Rows(count);
        var y = 0.0;
        for (var row = 0; row < rows.Count; row++)
        {
            for (var column = 0; column < count && row * count + column < Children.Count; column++)
            {
                Children[row * count + column].Arrange(new Rect(column * (width + Spacing), y, width, rows[row]));
            }
            y += rows[row] + RowSpacing;
        }
        return finalSize;
    }

    /// <summary>Each row's height: its tallest child's.</summary>
    private List<double> Rows(int count)
    {
        var rows = new List<double>();
        for (var start = 0; start < Children.Count; start += count)
        {
            rows.Add(Children.Skip(start).Take(count).Max(c => c.DesiredSize.Height));
        }
        return rows;
    }
}
