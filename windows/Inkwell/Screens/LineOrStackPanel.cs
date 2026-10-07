// A row's parts on one line where they fit, else one under another, each as wide as the row (the
// Mac's LineOrStack). On the line, every child takes its own width and the Fill child what is left;
// the row stacks when that would leave the Fill child under FillMinimum, or, with no Fill child,
// when the children together are wider than the row. A child's LineWidth is its width on the line
// only: stacked, it is as wide as the row (a fixed Width would stay fixed, or poke out). Settings'
// rows with a name, a text and its buttons use it, so the narrowest window (720 epx) clips none.
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace Inkwell.Screens;

public sealed partial class LineOrStackPanel : Panel
{
    /// <summary>Between children on the line.</summary>
    public double Spacing { get; set; } = 12;

    /// <summary>Between children stacked.</summary>
    public double StackSpacing { get; set; } = 6;

    /// <summary>The child that takes the line's spare width (-1: none).</summary>
    public int Fill { get; set; } = -1;

    /// <summary>The least width the Fill child gets on the line before the row stacks.</summary>
    public double FillMinimum { get; set; } = 160;

    /// <summary>A child's width on the line (unset: its own); stacked, every child is as wide as the row.</summary>
    public static readonly DependencyProperty LineWidthProperty = DependencyProperty.RegisterAttached(
        "LineWidth", typeof(double), typeof(LineOrStackPanel), new PropertyMetadata(double.NaN));

    public static double GetLineWidth(DependencyObject element) => (double)element.GetValue(LineWidthProperty);

    public static void SetLineWidth(DependencyObject element, double value) => element.SetValue(LineWidthProperty, value);

    private bool stacked;

    /// <summary>A child's width on the line, measuring it first when it has no LineWidth.</summary>
    private static double OnLine(UIElement child, bool measure)
    {
        var width = GetLineWidth(child);
        if (measure)
        {
            child.Measure(new Size(double.IsNaN(width) ? double.PositiveInfinity : width, double.PositiveInfinity));
        }
        return double.IsNaN(width) ? child.DesiredSize.Width : width;
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        var width = availableSize.Width;
        var visible = Visible();
        if (visible.Count == 0)
        {
            return new Size(0, 0);
        }
        // Each child's own width, on a line of unlimited width.
        double fixedWidth = 0;
        foreach (var (child, index) in visible)
        {
            if (index != Fill)
            {
                fixedWidth += OnLine(child, measure: true);
            }
        }
        var gaps = Spacing * (visible.Count - 1);
        var hasFill = visible.Exists(v => v.Index == Fill);
        var left = width - fixedWidth - gaps;
        stacked = double.IsFinite(width) && (hasFill ? left < FillMinimum : left < 0);
        if (stacked)
        {
            double height = 0;
            foreach (var (child, _) in visible)
            {
                child.Measure(new Size(width, double.PositiveInfinity));
                height += child.DesiredSize.Height;
            }
            return new Size(width, height + StackSpacing * (visible.Count - 1));
        }
        double tallest = 0;
        double used = fixedWidth + gaps;
        foreach (var (child, index) in visible)
        {
            if (index == Fill)
            {
                child.Measure(new Size(double.IsFinite(width) ? Math.Max(0, left) : double.PositiveInfinity, double.PositiveInfinity));
                used += child.DesiredSize.Width;
            }
            tallest = Math.Max(tallest, child.DesiredSize.Height);
        }
        return new Size(double.IsFinite(width) ? width : used, tallest);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var visible = Visible();
        if (stacked)
        {
            double y = 0;
            foreach (var (child, _) in visible)
            {
                child.Arrange(new Rect(0, y, finalSize.Width, child.DesiredSize.Height));
                y += child.DesiredSize.Height + StackSpacing;
            }
            return finalSize;
        }
        double fixedWidth = 0;
        foreach (var (child, index) in visible)
        {
            if (index != Fill)
            {
                fixedWidth += OnLine(child, measure: false);
            }
        }
        var fill = Math.Max(0, finalSize.Width - fixedWidth - Spacing * (visible.Count - 1));
        double x = 0;
        foreach (var (child, index) in visible)
        {
            var w = index == Fill ? fill : OnLine(child, measure: false);
            // The whole line's height, so each child's VerticalAlignment places it, as in a Grid row.
            child.Arrange(new Rect(x, 0, w, finalSize.Height));
            x += w + Spacing;
        }
        return finalSize;
    }

    private List<(UIElement Child, int Index)> Visible()
    {
        var list = new List<(UIElement, int)>(Children.Count);
        for (var i = 0; i < Children.Count; i++)
        {
            if (Children[i].Visibility == Visibility.Visible)
            {
                list.Add((Children[i], i));
            }
        }
        return list;
    }
}
