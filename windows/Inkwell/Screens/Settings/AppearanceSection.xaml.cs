// Settings > Appearance. It edits the mode shown now (GlowTheme.Dark): its preset and its two
// colours, as board 7 does; the other mode keeps its own. A colour is set when its picker closes,
// not at every step of a drag (one setting.set per choice). The section redraws when the
// appearance or the mode changes; nothing ticks.
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;

namespace Inkwell.Screens;

public sealed partial class AppearanceSection : UserControl
{
    private static readonly AppearanceMode[] Modes = [AppearanceMode.Light, AppearanceMode.Dark, AppearanceMode.System];

    private readonly GlowTheme theme;
    private readonly AppearanceModel appearance;
    private readonly Dictionary<string, ToggleButton> presets = [];
    private bool rendering;

    internal AppearanceSection(GlowTheme theme)
    {
        ArgumentNullException.ThrowIfNull(theme);
        this.theme = theme;
        appearance = theme.Appearance;
        InitializeComponent();
        var row = 0;
        for (var i = 0; i < GlowScheme.Presets.Count; i++)
        {
            if (i % 2 == 0)
            {
                Presets.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                row = i / 2;
            }
            var preset = GlowScheme.Presets[i];
            var button = PresetButton(preset);
            Grid.SetRow(button, row);
            Grid.SetColumn(button, i % 2);
            Presets.Children.Add(button);
            presets[preset.Id] = button;
        }
        theme.Changed += Render;
        Render();
    }

    private ToggleButton PresetButton(GlowPreset preset)
    {
        var dots = new Grid { Width = 42, Height = 26 };
        dots.Children.Add(Dot(GlowRgb.From(preset.You), 0));
        dots.Children.Add(Dot(GlowRgb.From(preset.Them), 16));
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        content.Children.Add(dots);
        content.Children.Add(new TextBlock { Text = preset.Name, VerticalAlignment = VerticalAlignment.Center });
        var button = new ToggleButton
        {
            Content = content,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            MinHeight = 48,
            Padding = new Thickness(12, 9, 12, 9),
            CornerRadius = new CornerRadius(14),
        };
        AutomationProperties.SetName(button, preset.Name);
        button.Click += (_, _) =>
        {
            if (!rendering)
            {
                appearance.SetDots(theme.Dark, preset.Id);
            }
        };
        return button;
    }

    /// <summary>A preset's dot: the colour, lit towards its partner shade at the upper left.</summary>
    private static Ellipse Dot(GlowRgb colour, double left)
    {
        var fill = new RadialGradientBrush { GradientOrigin = new Point(0.35, 0.35), Center = new Point(0.35, 0.35), RadiusX = 0.75, RadiusY = 0.75 };
        fill.GradientStops.Add(new GradientStop { Offset = 0, Color = GlowTheme.ColorOf(GlowScheme.Partner(colour)) });
        fill.GradientStops.Add(new GradientStop { Offset = 1, Color = GlowTheme.ColorOf(colour) });
        var dot = new Ellipse
        {
            Width = 26,
            Height = 26,
            Fill = fill,
            HorizontalAlignment = HorizontalAlignment.Left,
            Margin = new Thickness(left, 0, 0, 0),
            Opacity = left > 0 ? 0.9 : 1,
        };
        AutomationProperties.SetAccessibilityView(dot, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return dot;
    }

    private void Render()
    {
        rendering = true;
        try
        {
            var dark = theme.Dark;
            ModeChoice.SelectedIndex = Array.IndexOf(Modes, appearance.Mode);
            var modeName = dark ? "Dark" : "Light";
            DotsNote.Text = $"{modeName} keeps its own choice";
            var chosen = appearance.Dots(dark);
            foreach (var (id, button) in presets)
            {
                button.IsChecked = id == chosen;
            }
            ColoursHeading.Text = $"Colours in {modeName.ToLowerInvariant()} mode";
            ColourRows.Children.Clear();
            ColourRows.Children.Add(ColourRow(dark, you: true));
            ColourRows.Children.Add(ColourRow(dark, you: false));
            EdgeSwitch.IsOn = appearance.EdgeGlow;
            MotionChoice.SelectedIndex = appearance.Motion == AppearanceMotion.Still ? 1 : 0;
            FailureLine.Text = appearance.Failed ? AppearanceModel.FailedText : "";
            FailureLine.Visibility = appearance.Failed ? Visibility.Visible : Visibility.Collapsed;
        }
        finally
        {
            rendering = false;
        }
    }

    /// <summary>You or them in the mode shown: a swatch opening a picker, whose colour it is, and the way back to the preset's.</summary>
    private Grid ColourRow(bool dark, bool you)
    {
        var who = you ? "You" : "Them";
        var colour = appearance.Chosen(dark, you);
        var custom = appearance.Custom(dark, you) is not null;
        var row = new Grid
        {
            ColumnSpacing = 14,
            Padding = new Thickness(14, 10, 14, 10),
            CornerRadius = new CornerRadius(14),
            BorderThickness = new Thickness(1),
            BorderBrush = Parts.Brush("InkBorderBrush", this),
        };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var picker = new ColorPicker
        {
            Color = GlowTheme.ColorOf(colour),
            IsAlphaEnabled = false,
            IsMoreButtonVisible = false,
            IsColorChannelTextInputVisible = false,
        };
        var flyout = new Flyout { Content = picker };
        flyout.Closed += (_, _) =>
        {
            var picked = picker.Color;
            var rgb = new GlowRgb(picked.R / 255.0, picked.G / 255.0, picked.B / 255.0);
            if (rgb.Hex != colour.Hex)
            {
                appearance.SetCustom(dark, you, rgb);
            }
        };
        var swatch = new Button
        {
            Width = 44,
            Height = 32,
            Padding = new Thickness(0),
            MinWidth = 0,
            CornerRadius = new CornerRadius(8),
            Background = new SolidColorBrush(GlowTheme.ColorOf(colour)),
            BorderBrush = Parts.Brush("InkBorderBrush", this),
            Flyout = flyout,
        };
        AutomationProperties.SetName(swatch, you ? "Your colour" : "Their colour");
        AutomationProperties.SetHelpText(swatch, colour.Hex);
        row.Children.Add(swatch);

        var words = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        words.Children.Add(new TextBlock { Text = who, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Style = Parts.TextStyle("InkBodyStyle") });
        words.Children.Add(new TextBlock
        {
            Text = custom ? "Your own colour" : $"{GlowScheme.Preset(appearance.Dots(dark)).Name}'s",
            Style = Parts.TextStyle("InkCaptionStyle"),
        });
        Grid.SetColumn(words, 1);
        row.Children.Add(words);

        if (custom)
        {
            var reset = new HyperlinkButton { Content = "Use the preset's", VerticalAlignment = VerticalAlignment.Center };
            AutomationProperties.SetHelpText(reset, you ? "Your colour goes back to the preset's" : "Their colour goes back to the preset's");
            reset.Click += (_, _) => appearance.SetCustom(dark, you, null);
            Grid.SetColumn(reset, 2);
            row.Children.Add(reset);
        }
        return row;
    }

    private void OnModeChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && ModeChoice.SelectedIndex is >= 0 and < 3)
        {
            appearance.SetMode(Modes[ModeChoice.SelectedIndex]);
        }
    }

    private void OnEdgeToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering)
        {
            appearance.SetEdgeGlow(EdgeSwitch.IsOn);
        }
    }

    private void OnMotionChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && MotionChoice.SelectedIndex is 0 or 1)
        {
            appearance.SetMotion(MotionChoice.SelectedIndex == 1 ? AppearanceMotion.Still : AppearanceMotion.System);
        }
    }
}
