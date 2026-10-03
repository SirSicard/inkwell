// A panel whose children flow left to right and wrap to a new row between children, never inside
// one (WrapLayout places them). The Library's kind filters use it: in the list column's fixed
// width the last chip was cut off.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace Inkwell.Screens;

public sealed partial class WrapPanel : Panel
{
    /// <summary>Between children on a row.</summary>
    public double Spacing { get; set; }

    /// <summary>Between rows.</summary>
    public double RowSpacing { get; set; }

    protected override Size MeasureOverride(Size availableSize)
    {
        foreach (var child in Children)
        {
            child.Measure(new Size(availableSize.Width, double.PositiveInfinity));
        }
        var (_, width, height) = Place(availableSize.Width);
        return new Size(width, height);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var (places, _, _) = Place(finalSize.Width);
        for (var i = 0; i < Children.Count; i++)
        {
            var child = Children[i];
            child.Arrange(new Rect(places[i].X, places[i].Y, child.DesiredSize.Width, child.DesiredSize.Height));
        }
        return finalSize;
    }

    private (IReadOnlyList<(double X, double Y)> Places, double Width, double Height) Place(double width) =>
        WrapLayout.Place(Children.Select(c => (c.DesiredSize.Width, c.DesiredSize.Height)).ToList(), width, Spacing, RowSpacing);
}
