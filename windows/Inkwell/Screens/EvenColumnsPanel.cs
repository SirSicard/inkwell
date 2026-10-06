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

    protected override Size MeasureOverride(Size availableSize)
    {
        var (count, width) = StatsLayout.Columns(availableSize.Width, MinColumnWidth, Spacing, Children.Count);
        foreach (var child in Children)
        {
            child.Measure(new Size(width, double.PositiveInfinity));
        }
        var height = Rows(count).Sum() + RowSpacing * Math.Max(0, Rows(count).Count - 1);
        var total = double.IsInfinity(availableSize.Width) ? width * count + Spacing * (count - 1) : availableSize.Width;
        return new Size(Math.Max(0, total), height);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var (count, width) = StatsLayout.Columns(finalSize.Width, MinColumnWidth, Spacing, Children.Count);
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
