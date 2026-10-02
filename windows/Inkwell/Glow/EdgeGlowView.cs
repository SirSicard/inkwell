// The window's edge glow (the canvas's edge layer), drawn natively: the tokens' strokes as borders
// round the window, wide and faint to narrow and strong, all with one horizontal gradient
// (GlowEdgeGradient's stops).
// It sits above the content and takes no clicks. It changes only when the orb draws a frame
// (InkSurface.Drawn: live on the shared clock, else one still frame) or the appearance changes, so
// it moves exactly when the orb does and never ticks on its own. Off when the user turns it off;
// still when the orb is still. Decorative: Narrator skips it.
using Inkwell.Core.Glow;
using Inkwell.Ink;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Color = Windows.UI.Color;

namespace Inkwell;

public sealed partial class EdgeGlowView : Grid
{
    /// <summary>Windows 11's window corner.</summary>
    private const double WindowCorner = 8;

    private readonly LinearGradientBrush gradient = new() { StartPoint = new Point(0, 0.5), EndPoint = new Point(1, 0.5) };
    private readonly GradientStop[] stops = new GradientStop[5];
    private GlowFrame? frame;
    private GlowColours colours = GlowScheme.Resolve(false, GlowScheme.DefaultPreset);
    private bool enabled = true;

    public EdgeGlowView()
    {
        IsHitTestVisible = false;
        AutomationProperties.SetAccessibilityView(this, AccessibilityView.Raw);
        for (var i = 0; i < stops.Length; i++)
        {
            stops[i] = new GradientStop();
            gradient.GradientStops.Add(stops[i]);
        }
        // The canvas strokes its edge from the middle of each line, so half of each stroke shows
        // inside the window: a border of half the stroke's width.
        foreach (var stroke in GlowTokens.EdgeGlow.Strokes)
        {
            Children.Add(new Border
            {
                BorderThickness = new Thickness(stroke.Width / 2),
                CornerRadius = new CornerRadius(WindowCorner),
                BorderBrush = gradient,
                Opacity = stroke.Alpha,
            });
        }
        Visibility = Visibility.Collapsed;
    }

    /// <summary>UI thread. A frame the orb drew.</summary>
    internal void Show(GlowFrame drawn)
    {
        frame = drawn;
        Render();
    }

    /// <summary>UI thread. The colours, and whether the user wants the glow at all.</summary>
    internal void Set(GlowColours resolved, bool on)
    {
        colours = resolved;
        enabled = on;
        Render();
    }

    private void Render()
    {
        Span<GlowEdgeStop> computed = stackalloc GlowEdgeStop[5];
        if (!enabled || frame is not { } f
            || !GlowEdgeGradient.Stops(f.Dictating, f.Meeting, f.Blotting, f.You, f.Them, f.Moving ? f.Time : 0, colours, computed))
        {
            Visibility = Visibility.Collapsed;
            return;
        }
        for (var i = 0; i < stops.Length; i++)
        {
            var stop = computed[i];
            stops[i].Offset = stop.Offset;
            var (r, g, b) = stop.Colour.Bytes;
            stops[i].Color = Color.FromArgb((byte)Math.Round(Math.Clamp(stop.Alpha, 0, 1) * 255), r, g, b);
        }
        Visibility = Visibility.Visible;
    }
}
