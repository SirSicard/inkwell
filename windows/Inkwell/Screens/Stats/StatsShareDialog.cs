// The share card, as the Mac's ShareCardSheet: the card as it will look, the ticks for what it
// carries, and Copy or Save. Numbers only, made on this PC; nothing leaves it unless the user
// pastes or saves the image somewhere.
//
// The card is drawn in the mode's own colours (fixed tokens, not the window's brushes, so the image
// is the same wherever it is made) at twice its size in pixels: it is laid out at 2 ÷ the screen's
// scale, so RenderTargetBitmap's pixels are 2× the card's 520 points at any display scaling, and
// the PNG says 144 dpi so it pastes at the card's size. It is drawn from a copy outside the
// dialog's view, never through a scaled preview: under a Viewbox it was rasterised at the
// preview's size and came out blurred. The preview shows the PNG itself, as the Mac's does. Copy
// puts only that image on the clipboard (the PNG, and a bitmap for apps that take only that);
// Save asks where with Windows' save picker.
using System.Runtime.InteropServices.WindowsRuntime;
using Inkwell.Core.Events;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics.Imaging;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;

namespace Inkwell.Screens;

internal sealed class StatsShareDialog
{
    /// <summary>The card's width in points, and pixels per point in the image.</summary>
    public const double CardWidth = 520;
    public const double Scale = 2;

    /// <summary>
    /// The card's points are typographic (72 to the inch): at Scale pixels each, the PNG says 144
    /// dpi, and pastes 520 points wide (as the Mac's does).
    /// </summary>
    private const double PointsPerInch = 72;

    /// <summary>The preview's share of the card's width.</summary>
    private const double PreviewShare = 0.6;

    private readonly StatsModel stats;
    private readonly GlowTheme theme;
    private readonly Func<nint> windowHandle;
    private readonly ContentDialog dialog;
    private readonly HashSet<ShareStat> selected = [.. ShareStats.Defaults];
    /// <summary>Where the card is drawn: in the dialog's tree (RenderTargetBitmap draws only what is), out of its view.</summary>
    private readonly Canvas cardHost = new() { Width = 0, Height = 0 };
    private readonly Image preview;
    private readonly TextBlock empty;
    private readonly StackPanel ticks = new() { Spacing = 8 };
    private readonly TextBlock status;
    private readonly Button save;
    private readonly Button copy;
    private byte[]? png;
    private int renders;

    public StatsShareDialog(StatsModel stats, GlowTheme theme, Func<nint> windowHandle, XamlRoot root, ElementTheme mode)
    {
        this.stats = stats;
        this.theme = theme;
        this.windowHandle = windowHandle;
        preview = new Image { Width = CardWidth * PreviewShare, Stretch = Stretch.Uniform };
        empty = new TextBlock
        {
            Text = "Tick a number to put it on the card.",
            Style = Parts.TextStyle("InkBodyStyle"),
            Width = CardWidth * PreviewShare,
            Height = 160,
        };
        status = new TextBlock { Style = Parts.TextStyle("InkCaptionStyle"), VerticalAlignment = VerticalAlignment.Center, TextWrapping = TextWrapping.Wrap };
        // Off until there is an image to copy or save.
        save = new Button { Content = "Save…", IsEnabled = false };
        save.Click += async (_, _) => await Save();
        copy = new Button { Content = "Copy", Style = Parts.TextStyle("InkAccentButtonStyle"), IsEnabled = false };
        copy.Click += async (_, _) => await Copy();

        var heading = new TextBlock { Text = "Share card", Style = Parts.TextStyle("InkHeadingStyle") };
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level1);
        var body = new StackPanel { Spacing = 16 };
        body.Children.Add(cardHost);
        body.Children.Add(heading);
        body.Children.Add(new TextBlock
        {
            Text = "Numbers only, made on this PC. Nothing leaves it unless you paste or save the image somewhere.",
            Style = Parts.TextStyle("InkCaptionStyle"),
        });
        var columns = new Grid { ColumnSpacing = 24 };
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var shown = new Grid { VerticalAlignment = VerticalAlignment.Top };
        shown.Children.Add(preview);
        shown.Children.Add(empty);
        columns.Children.Add(shown);
        Grid.SetColumn(ticks, 1);
        columns.Children.Add(ticks);
        body.Children.Add(columns);
        var actions = new Grid { ColumnSpacing = 10 };
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        actions.Children.Add(status);
        Grid.SetColumn(save, 1);
        Grid.SetColumn(copy, 2);
        actions.Children.Add(save);
        actions.Children.Add(copy);
        body.Children.Add(actions);

        dialog = new ContentDialog
        {
            XamlRoot = root,
            RequestedTheme = mode,
            Content = body,
            CloseButtonText = "Done",
            DefaultButton = ContentDialogButton.Close,
        };
        // As wide as the Mac's sheet: the preview beside the ticks.
        dialog.Resources["ContentDialogMaxWidth"] = 720.0;
        // The ticks, or new numbers counted while it is open; the mode; your colours.
        stats.PropertyChanged += OnStatsChanged;
        theme.Changed += OnThemeChanged;
        dialog.Closed += (_, _) =>
        {
            stats.PropertyChanged -= OnStatsChanged;
            theme.Changed -= OnThemeChanged;
        };
        dialog.Opened += (_, _) => Rebuild();
    }

    public Windows.Foundation.IAsyncOperation<ContentDialogResult> ShowAsync() => dialog.ShowAsync();

    private void OnStatsChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Rebuild();

    private void OnThemeChanged() => Rebuild();

    /// <summary>What the ticks and the card were last made from: only new numbers, a new mode or new colours make them again (a reload that changes nothing would reset the ticks and the focus).</summary>
    private (StatsCounted Counted, bool Dark, GlowColours Colours)? built;

    private IReadOnlyList<ShareLine> Lines() =>
        stats.Counted is { } counted ? ShareStats.Lines(counted, selected, stats.Culture) : [];

    private void Rebuild()
    {
        if (stats.Counted is not { } counted)
        {
            return;
        }
        var from = (counted, theme.Dark, theme.Colours);
        if (built == from)
        {
            return;
        }
        built = from;
        ticks.Children.Clear();
        foreach (var stat in ShareStats.All)
        {
            var available = stat.Available(counted);
            var box = new CheckBox
            {
                Content = stat.TickTitle(counted),
                IsChecked = selected.Contains(stat) && available,
                IsEnabled = available,
            };
            box.Checked += (_, _) => Tick(stat, true);
            box.Unchecked += (_, _) => Tick(stat, false);
            ticks.Children.Add(box);
        }
        _ = Render();
    }

    private void Tick(ShareStat stat, bool on)
    {
        if (on)
        {
            selected.Add(stat);
        }
        else
        {
            selected.Remove(stat);
        }
        Report(null, failed: false);
        _ = Render();
    }

    /// <summary>Draws the card and makes its PNG, which Copy and Save use.</summary>
    private async Task Render()
    {
        var mine = ++renders;
        var lines = Lines();
        AutomationProperties.SetName(preview, ShareStats.Spoken(lines));
        AutomationProperties.SetName(empty, ShareStats.Spoken(lines));
        preview.Visibility = lines.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
        empty.Visibility = lines.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        png = null;
        save.IsEnabled = false;
        copy.IsEnabled = false;
        cardHost.Children.Clear();
        if (lines.Count == 0 || cardHost.XamlRoot is not { } root)
        {
            return;
        }
        // Laid out so that its pixels are Scale × its points at this screen's scale, far to the
        // left of the dialog, where nobody sees it.
        var unit = Scale / root.RasterizationScale;
        var card = Card(lines, theme.Dark, theme.Colours.You, theme.Colours.Them, unit);
        Canvas.SetLeft(card, -CardWidth * unit * 4);
        cardHost.Children.Add(card);
        try
        {
            // Measured and drawn first: RenderTargetBitmap takes what is laid out.
            card.UpdateLayout();
            var bitmap = new RenderTargetBitmap();
            await bitmap.RenderAsync(card);
            var pixels = await bitmap.GetPixelsAsync();
            var made = await Encode(pixels.ToArray(), (uint)bitmap.PixelWidth, (uint)bitmap.PixelHeight);
            if (mine != renders)
            {
                return;
            }
            var shown = new BitmapImage();
            using (var stream = new InMemoryRandomAccessStream())
            {
                await stream.WriteAsync(made.AsBuffer());
                stream.Seek(0);
                await shown.SetSourceAsync(stream);
            }
            if (mine != renders)
            {
                return;
            }
            // Copy and Save once the preview shows what they would give.
            preview.Source = shown;
            png = made;
            save.IsEnabled = true;
            copy.IsEnabled = true;
        }
        catch (Exception e)
        {
            // Said, never an earlier card beside the new ticks; the cause is logged by name.
            if (mine == renders)
            {
                preview.Source = null;
                InkLog.Write($"couldn't make the share card: {e.GetType().Name}");
                Report("Couldn't make the image.", failed: true);
            }
        }
    }

    /// <summary>BGRA pixels as a PNG that says 144 dpi.</summary>
    private static async Task<byte[]> Encode(byte[] bgra, uint width, uint height)
    {
        using var stream = new InMemoryRandomAccessStream();
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, stream);
        encoder.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, width, height, PointsPerInch * Scale, PointsPerInch * Scale, bgra);
        await encoder.FlushAsync();
        var bytes = new byte[stream.Size];
        stream.Seek(0);
        using var reader = new DataReader(stream.GetInputStreamAt(0));
        await reader.LoadAsync((uint)stream.Size);
        reader.ReadBytes(bytes);
        return bytes;
    }

    private async Task Copy()
    {
        if (png is null)
        {
            return;
        }
        try
        {
            var stream = new InMemoryRandomAccessStream();
            await stream.WriteAsync(png.AsBuffer());
            stream.Seek(0);
            // Only the image: no text, no file. The PNG keeps the card's round corners clear; the
            // bitmap is for apps that take nothing else.
            var package = new DataPackage();
            package.SetData("PNG", stream);
            package.SetBitmap(RandomAccessStreamReference.CreateFromStream(stream));
            Clipboard.SetContent(package);
            Clipboard.Flush();
            Report("Copied. Paste it wherever you like.", failed: false);
        }
        catch (Exception e) when (e is System.Runtime.InteropServices.COMException or UnauthorizedAccessException or InvalidOperationException)
        {
            Report("Couldn't copy the image.", failed: true);
        }
    }

    private async Task Save()
    {
        // The image as it is now: the card may be drawn again while the picker is open (new
        // numbers, the mode), which clears what Copy and Save use until it is done.
        if (png is not { } bytes)
        {
            return;
        }
        var picker = new FileSavePicker { SuggestedFileName = System.IO.Path.GetFileNameWithoutExtension(ShareStats.FileName) };
        picker.FileTypeChoices.Add("PNG image", [".png"]);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, windowHandle());
        StorageFile? file;
        try
        {
            file = await picker.PickSaveFileAsync();
        }
        catch (Exception e) when (e is System.Runtime.InteropServices.COMException or UnauthorizedAccessException)
        {
            Report($"Couldn't save it: {e.Message}", failed: true);
            return;
        }
        // Cancelled: nothing to say.
        if (file is null)
        {
            return;
        }
        try
        {
            await FileIO.WriteBytesAsync(file, bytes);
            Report("Saved.", failed: false);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or System.Runtime.InteropServices.COMException)
        {
            Report($"Couldn't save it: {e.Message}", failed: true);
        }
    }

    /// <summary>What Copy or Save did, shown and read aloud.</summary>
    private void Report(string? text, bool failed)
    {
        status.Text = text ?? "";
        status.Foreground = Parts.Brush(failed ? "InkAlertBrush" : "InkSecondaryTextBrush", status);
        // A peer made if none exists yet (no screen reader has asked for one): the line is read.
        if (text is not null && (FrameworkElementAutomationPeer.FromElement(copy) ?? FrameworkElementAutomationPeer.CreatePeerForElement(copy)) is { } peer)
        {
            peer.RaiseNotificationEvent(AutomationNotificationKind.ActionCompleted, AutomationNotificationProcessing.ImportantMostRecent, text, "stats-share");
        }
    }

    /// <summary>
    /// The card itself, at <paramref name="unit"/> pixels per point: "Inkwell", each number over its
    /// label, the foot, on the mode's background with your two colours as a soft glow in its corner.
    /// </summary>
    public static Border Card(IReadOnlyList<ShareLine> lines, bool dark, GlowRgb you, GlowRgb them, double unit)
    {
        ArgumentNullException.ThrowIfNull(lines);
        var mode = GlowScheme.Palette(dark);
        var text = new SolidColorBrush(GlowTheme.ColorOf(GlowRgb.From(mode.Text)));
        var secondary = new SolidColorBrush(GlowTheme.ColorOf(GlowRgb.From(mode.Secondary)));
        var display = Parts.Font("InkDisplayFontFamily");
        var face = Parts.Font("InkInterfaceFontFamily");
        var content = new StackPanel { Spacing = 18 * unit, Padding = new Thickness(36 * unit) };
        content.Children.Add(new TextBlock { Text = "Inkwell", FontFamily = display, FontSize = 22 * unit, Foreground = text });
        var numbers = new StackPanel { Spacing = 14 * unit };
        foreach (var line in lines)
        {
            var one = new StackPanel { Spacing = 2 * unit };
            one.Children.Add(new TextBlock { Text = line.Value, FontFamily = display, FontSize = 34 * unit, Foreground = text });
            one.Children.Add(new TextBlock { Text = line.Label, FontFamily = face, FontSize = 14 * unit, Foreground = secondary });
            numbers.Children.Add(one);
        }
        content.Children.Add(numbers);
        content.Children.Add(new TextBlock { Text = ShareStats.Foot, FontFamily = face, FontSize = 12 * unit, Foreground = secondary });
        var glow = new Ellipse
        {
            Width = 340 * unit,
            Height = 340 * unit,
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Top,
            // The Mac's circle sits 110 right and 120 up of the corner's.
            Margin = new Thickness(0, -120 * unit, -110 * unit, 0),
            Fill = new RadialGradientBrush
            {
                GradientStops =
                {
                    new GradientStop { Color = GlowTheme.ColorOf(you, (byte)Math.Round(0.55 * 255)), Offset = 0 },
                    new GradientStop { Color = GlowTheme.ColorOf(them, (byte)Math.Round(0.3 * 255)), Offset = 0.5 },
                    new GradientStop { Color = GlowTheme.ColorOf(them, 0), Offset = 1 },
                },
            },
        };
        var layers = new Grid();
        layers.Children.Add(glow);
        layers.Children.Add(content);
        return new Border
        {
            Width = CardWidth * unit,
            Background = new SolidColorBrush(GlowTheme.ColorOf(GlowRgb.From(mode.Background))),
            CornerRadius = new CornerRadius(22 * unit),
            Child = layers,
        };
    }
}
