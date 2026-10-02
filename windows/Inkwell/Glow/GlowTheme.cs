// The window's look: the appearance settings (AppearanceModel) applied. The mode sets the window's
// RequestedTheme (light, dark, or the system's), which picks the token dictionaries App.xaml names;
// the you and them colours of the mode shown are resolved (GlowScheme.Resolve) into the two brushes the
// dots and lanes use, the orb's colours, the Drop's pill and the tray's state dots; the caption
// buttons follow the mode. Every surface listens to Changed. Nothing here polls: it follows the
// settings, the window's theme (Windows' app mode, while the mode is the system's) and High Contrast.
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;

namespace Inkwell;

internal sealed class GlowTheme
{
    private readonly AppearanceModel appearance;
    private readonly AccessibilitySettings accessibility = new();
    private FrameworkElement? root;
    private AppWindow? window;

    /// <summary>UI thread. Follows <paramref name="appearance"/> and Windows' High Contrast.</summary>
    public GlowTheme(AppearanceModel appearance, DispatcherQueue ui)
    {
        ArgumentNullException.ThrowIfNull(appearance);
        ArgumentNullException.ThrowIfNull(ui);
        this.appearance = appearance;
        appearance.PropertyChanged += (_, _) => Update();
        // Raised off the UI thread.
        accessibility.HighContrastChanged += (_, _) => ui.TryEnqueue(Update);
        Update();
    }

    /// <summary>The appearance settings it applies.</summary>
    public AppearanceModel Appearance => appearance;

    /// <summary>Whether the window shows night.</summary>
    public bool Dark { get; private set; }

    /// <summary>Whether Windows' High Contrast is on: solid cards, a dimmed orb.</summary>
    public bool HighContrast { get; private set; }

    /// <summary>The mode's resolved colours.</summary>
    public GlowColours Colours { get; private set; } = GlowScheme.Resolve(false, GlowScheme.DefaultPreset);

    /// <summary>The orb's colours.</summary>
    public GlowLook Look { get; private set; } = GlowLook.Default;

    /// <summary>The Drop's pill.</summary>
    public DropLook DropLook { get; private set; } = DropLook.Default;

    /// <summary>Whether the window's edge glows while something is live.</summary>
    public bool EdgeGlow => appearance.EdgeGlow;

    /// <summary>The user's Always still.</summary>
    public bool AlwaysStill => appearance.Motion == AppearanceMotion.Still;

    /// <summary>Anything above changed. UI thread.</summary>
    public event Action? Changed;

    /// <summary>UI thread, once: the window whose content and caption buttons follow the mode.</summary>
    public void Attach(FrameworkElement content, AppWindow appWindow)
    {
        ArgumentNullException.ThrowIfNull(content);
        ArgumentNullException.ThrowIfNull(appWindow);
        root = content;
        window = appWindow;
        // Windows' app mode changed while the mode is the system's.
        content.ActualThemeChanged += (_, _) => Update();
        Update();
    }

    private void Update()
    {
        HighContrast = accessibility.HighContrast;
        if (root is not null)
        {
            var requested = appearance.Mode switch
            {
                AppearanceMode.Light => ElementTheme.Light,
                AppearanceMode.Dark => ElementTheme.Dark,
                _ => ElementTheme.Default,
            };
            if (root.RequestedTheme != requested)
            {
                root.RequestedTheme = requested;
            }
            Dark = root.ActualTheme == ElementTheme.Dark;
        }
        else
        {
            Dark = appearance.IsDark(Application.Current.RequestedTheme == ApplicationTheme.Dark);
        }
        Colours = appearance.Colours(Dark);
        Look = new GlowLook(Dark, Tuple(Colours.You), Tuple(Colours.YouPartner), Tuple(Colours.Them), Tuple(Colours.ThemPartner),
            Tuple(Colours.Idle), Tuple(Colours.Ink));
        var tokens = GlowScheme.Palette(Dark);
        DropLook = new DropLook(Dark, Tuple(tokens.Background), Tuple(tokens.Border), Tuple(tokens.Text), Tuple(tokens.Secondary),
            Tuple(tokens.Alert), Tuple(tokens.ButtonFill), Tuple(tokens.ButtonLabel));
        SetBrush("GlowYouBrush", Colours.You);
        SetBrush("GlowThemBrush", Colours.Them);
        CaptionButtons(tokens);
        Changed?.Invoke();
    }

    /// <summary>The caption buttons (minimise, maximise, close) drawn in the mode's text colour on no fill.</summary>
    private void CaptionButtons(GlowPalette tokens)
    {
        if (window is null || !AppWindowTitleBar.IsCustomizationSupported())
        {
            return;
        }
        var bar = window.TitleBar;
        var clear = Color.FromArgb(0, 0, 0, 0);
        bar.ButtonBackgroundColor = clear;
        bar.ButtonInactiveBackgroundColor = clear;
        var text = GlowRgb.From(tokens.Text);
        bar.ButtonForegroundColor = ColorOf(text);
        bar.ButtonInactiveForegroundColor = ColorOf(GlowRgb.From(tokens.Secondary));
        bar.ButtonHoverForegroundColor = ColorOf(text);
        bar.ButtonHoverBackgroundColor = ColorOf(text, 0x18);
        bar.ButtonPressedForegroundColor = ColorOf(text);
        bar.ButtonPressedBackgroundColor = ColorOf(text, 0x28);
    }

    private static void SetBrush(string key, GlowRgb colour)
    {
        if (Application.Current.Resources.TryGetValue(key, out var found) && found is SolidColorBrush brush)
        {
            brush.Color = ColorOf(colour);
        }
    }

    public static Color ColorOf(GlowRgb c, byte alpha = 0xFF)
    {
        var (r, g, b) = c.Bytes;
        return Color.FromArgb(alpha, r, g, b);
    }

    private static (float R, float G, float B) Tuple(GlowRgb c) => ((float)c.R, (float)c.G, (float)c.B);

    private static (float R, float G, float B) Tuple(GlowColor c) => Tuple(GlowRgb.From(c));
}
