// A Settings row's name and what goes with it (the Mac's SettingColumns): side by side, the name in
// its column and the controls beside it, where the row has room for both; narrower, the name above
// the controls, each as wide as the row. ModesLayout decides and places, so the choice is tested
// headless; this measures the two children and puts them there.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace Inkwell.Screens;

public sealed partial class SettingColumnsPanel : Panel
{
    /// <summary>The name's column, side by side.</summary>
    public double TitleColumn { get; set; } = ModesLayout.TitleColumn;

    protected override Size MeasureOverride(Size availableSize)
    {
        if (Children.Count != 2)
        {
            return new Size(0, 0);
        }
        var width = availableSize.Width;
        var stacked = ModesLayout.Stacks(width, TitleColumn);
        Children[0].Measure(new Size(stacked ? width : TitleColumn, double.PositiveInfinity));
        Children[1].Measure(new Size(ModesLayout.ControlsWidth(width, TitleColumn), double.PositiveInfinity));
        var arrangement = ModesLayout.Arrange(width, Children[0].DesiredSize.Height, Children[1].DesiredSize.Height, TitleColumn);
        var measured = double.IsFinite(width)
            ? width
            : TitleColumn + ModesLayout.ColumnSpacing + Children[1].DesiredSize.Width;
        return new Size(measured, arrangement.Height);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        if (Children.Count != 2)
        {
            return finalSize;
        }
        var arrangement = ModesLayout.Arrange(finalSize.Width, Children[0].DesiredSize.Height, Children[1].DesiredSize.Height, TitleColumn);
        Place(Children[0], arrangement.Title);
        Place(Children[1], arrangement.Controls);
        return new Size(finalSize.Width, arrangement.Height);
    }

    private static void Place(UIElement child, (double X, double Y, double Width, double Height) place) =>
        child.Arrange(new Rect(place.X, place.Y, Math.Max(0, place.Width), Math.Max(0, place.Height)));
}
