// A milestone reached, in the main window (MilestoneCelebration): the glow, over the orb wherever it
// has wandered, and the one line at the window's foot, as the Mac's MilestoneGlow and MilestoneNote.
//
// The glow holds the orb still (InkPanel.Hold), waits for any glide in progress to finish, reads the
// orb's spot, lights there (in over 0.8 s, held 0.6 s, out over 1.2 s) and lets the orb go when it
// has faded. It plays once per celebration (StatsModel.BeginGlow), only while the window is on
// screen, never under Always still or with animation effects off. It is a composition visual handed
// to the glow layer, faded on the compositor: a XAML shape there was drawn under the orb's
// swapchain (it showed only through the cards' acrylic). The line shows for 6 s while the
// window is on screen and is announced to Narrator once (StatsModel.BeginAnnouncement); off screen
// it waits, and shows again when the window is back. Nothing here runs at rest: the glow's element
// is hidden and the timers exist only while a celebration shows.
using System.ComponentModel;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using System.Numerics;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Media;

namespace Inkwell;

// The window holds it for its life; each glow's token source is cancelled when it ends early.
[System.Diagnostics.CodeAnalysis.SuppressMessage("Reliability", "CA1001", Justification = "Lives as long as the window")]
internal sealed class MilestoneView
{
    private readonly StatsModel stats;
    private readonly WindowPresence presence;
    private readonly GlowTheme theme;
    private readonly InkPanel orb;
    private readonly Canvas glowLayer;
    private readonly Compositor compositor;
    private readonly SpriteVisual glow;
    private readonly Border note;
    private readonly TextBlock noteText;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer noteTimer;
    /// <summary>The celebration shown now, and what cancels its glow.</summary>
    private int? shownSerial;
    private CancellationTokenSource? glowing;

    /// <param name="glowLayer">Over the orb, under the screens: where the glow is drawn.</param>
    /// <param name="noteHost">Over the screens: where the line sits, at the foot.</param>
    public MilestoneView(StatsModel stats, WindowPresence presence, GlowTheme theme, InkPanel orb, Canvas glowLayer, Grid noteHost)
    {
        this.stats = stats;
        this.presence = presence;
        this.theme = theme;
        this.orb = orb;
        this.glowLayer = glowLayer;
        compositor = ElementCompositionPreview.GetElementVisual(glowLayer).Compositor;
        glow = compositor.CreateSpriteVisual();
        glow.Opacity = 0;
        ElementCompositionPreview.SetElementChildVisual(glowLayer, glow);
        noteText = new TextBlock
        {
            Style = (Style)Application.Current.Resources["InkBodyStyle"],
            TextWrapping = TextWrapping.NoWrap,
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetLiveSetting(noteText, AutomationLiveSetting.Polite);
        var dismiss = new Button
        {
            Width = 32,
            Height = 32,
            Padding = new Thickness(0),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0),
            Content = new FontIcon { Glyph = "", FontSize = 12 },
        };
        AutomationProperties.SetName(dismiss, MilestoneCelebration.DismissName);
        dismiss.Click += (_, _) =>
        {
            if (shownSerial is int serial)
            {
                stats.DismissCelebration(serial);
            }
        };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        // Segoe Fluent's star: the Mac's sparkle, decorative.
        row.Children.Add(new FontIcon { Glyph = "", FontSize = 14, VerticalAlignment = VerticalAlignment.Center, IsTabStop = false });
        AutomationProperties.SetAccessibilityView(row.Children[0], AccessibilityView.Raw);
        row.Children.Add(noteText);
        row.Children.Add(dismiss);
        note = new Border
        {
            Style = (Style)Application.Current.Resources["InkCardStyle"],
            Padding = new Thickness(18, 8, 10, 8),
            MaxWidth = 520,
            Margin = new Thickness(0, 0, 0, 24),
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Bottom,
            Child = row,
            Visibility = Visibility.Collapsed,
        };
        noteHost.Children.Add(note);
        noteTimer = noteHost.DispatcherQueue.CreateTimer();
        noteTimer.IsRepeating = false;
        noteTimer.Interval = MilestoneCelebration.Shown;
        noteTimer.Tick += (_, _) =>
        {
            if (shownSerial is int serial)
            {
                stats.DismissCelebration(serial);
            }
        };
        stats.PropertyChanged += OnChanged;
        presence.PropertyChanged += OnChanged;
        Update();
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Update();

    private void Update()
    {
        var showing = MilestoneCelebration.Showing(stats.Pending, presence.OnScreen);
        if (showing?.Serial == shownSerial)
        {
            return;
        }
        // Off screen, dismissed, or replaced by a later one: the glow goes out at once and the line
        // goes. Off screen the celebration stays pending, and shows again when the window is back.
        noteTimer.Stop();
        glowing?.Cancel();
        glowing = null;
        shownSerial = showing?.Serial;
        if (showing is null)
        {
            note.Visibility = Visibility.Collapsed;
            return;
        }
        noteText.Text = showing.Note;
        note.Visibility = Visibility.Visible;
        if (stats.BeginAnnouncement(showing.Serial)
            && FrameworkElementAutomationPeer.FromElement(noteText) is { } peer)
        {
            peer.RaiseNotificationEvent(AutomationNotificationKind.Other, AutomationNotificationProcessing.ImportantMostRecent, showing.Note, "milestone");
        }
        noteTimer.Start();
        if (MilestoneCelebration.Glows(theme.AlwaysStill, !SystemMotion.AnimationsEnabled))
        {
            glowing = new CancellationTokenSource();
            _ = Glow(showing.Serial, glowing.Token);
        }
    }

    /// <summary>Holds the orb, lights the glow on it once it has arrived, fades it, and lets the orb go.</summary>
    private async Task Glow(int serial, CancellationToken cancel)
    {
        try
        {
            var (x, y) = await orb.Hold(cancel);
            if (cancel.IsCancellationRequested || !stats.BeginGlow(serial))
            {
                return;
            }
            Place(x, y);
            Fade((float)MilestoneCelebration.GlowPeak, MilestoneCelebration.GlowIn, compositor.CreateCubicBezierEasingFunction(new Vector2(0, 0), new Vector2(0.58f, 1)));
            await Task.Delay(MilestoneCelebration.GlowIn + MilestoneCelebration.GlowHeld, cancel);
            Fade(0, MilestoneCelebration.GlowOut, compositor.CreateCubicBezierEasingFunction(new Vector2(0.42f, 0), new Vector2(1, 1)));
            // Holds the orb through the fade.
            await Task.Delay(MilestoneCelebration.GlowOut, cancel);
        }
        catch (TaskCanceledException)
        {
            // Out at once (the window left the screen, the line was dismissed).
        }
        finally
        {
            if (cancel.IsCancellationRequested)
            {
                glow.StopAnimation(nameof(glow.Opacity));
                glow.Opacity = 0;
            }
            orb.Release();
        }
    }

    /// <summary>The glow centred on the orb's spot (fractions of the window), the orb's size, in your colour fading to theirs.</summary>
    private void Place(double x, double y)
    {
        var (w, h) = (glowLayer.ActualWidth, glowLayer.ActualHeight);
        var size = (float)(Math.Min(w, h) * GlowTokens.Orb.Main.Unit * MilestoneCelebration.GlowUnits);
        glow.Size = new Vector2(size, size);
        glow.Offset = new Vector3((float)(w * x) - size / 2, (float)(h * y) - size / 2, 0);
        var colours = theme.Colours;
        // Your colour at the centre, theirs halfway out, nothing at the edge: a round glow in a
        // square visual.
        var brush = compositor.CreateRadialGradientBrush();
        brush.ColorStops.Add(compositor.CreateColorGradientStop(0, GlowTheme.ColorOf(colours.You, (byte)Math.Round(0.9 * 255))));
        brush.ColorStops.Add(compositor.CreateColorGradientStop(0.5f, GlowTheme.ColorOf(colours.Them, (byte)Math.Round(0.5 * 255))));
        brush.ColorStops.Add(compositor.CreateColorGradientStop(1, GlowTheme.ColorOf(colours.Them, 0)));
        glow.Brush = brush;
    }

    /// <summary>The glow's opacity to <paramref name="to"/> over <paramref name="over"/>, on the compositor.</summary>
    private void Fade(float to, TimeSpan over, CompositionEasingFunction ease)
    {
        var animation = compositor.CreateScalarKeyFrameAnimation();
        animation.InsertKeyFrame(1, to, ease);
        animation.Duration = over;
        glow.StartAnimation(nameof(glow.Opacity), animation);
    }
}
