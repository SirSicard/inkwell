// The orb in the app's window: a SwapChainPanel behind all the content, holding a composition
// swapchain that Direct3D 11 draws into, on the shared clock while live and once per change
// otherwise (InkSurface). Each frame it draws is passed on (Drawn): the edge glow follows it.
//
// A SwapChainPanel shows nothing of the XAML behind it: where its swapchain is transparent, WinUI
// shows black (a 50% red clear showed as #7F0000 over a #121118 window). So the panel paints its
// own backdrop, the colour of what it sits on (Backdrop, a theme resource that follows the mode),
// and the orb is blended over it.
//
// It counts as on screen while its window is shown (XamlRoot.IsHostVisible), it is visible and it
// has a size: hidden to the tray, the window's ink draws nothing.
//
// The main window's orb wanders (WanderBounds, InkSurface): the window asks it to move at rest
// when its screen or monitor changes (MoveAtRest) and when it is activated (Activated). Something
// drawn over the orb (a milestone's glow) holds it with Hold and reads where it is.
using Inkwell.Core.Glow;
using Inkwell.Ink;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace Inkwell;

// The surface and the swapchain are released on Unloaded (Detach); a XAML element is never disposed.
[System.Diagnostics.CodeAnalysis.SuppressMessage("Reliability", "CA1001", Justification = "Released on Unloaded")]
public sealed partial class InkPanel : SwapChainPanel, IInkTarget
{
    private InkSurface? surface;
    private CompositionSwapChain? swapChain;
    private XamlRoot? root;
    private InkState state = InkState.Idle;
    private GlowLook look = GlowLook.Default;
    private bool alwaysStill;
    /// <summary>The orb's opacity over its backdrop, fading toward what the window asks (OrbOpacity).</summary>
    private readonly OrbFade fade = new();
    /// <summary>Draws the fade's end once it is reached, when no frame of the ink would (a still orb).</summary>
    private Microsoft.UI.Dispatching.DispatcherQueueTimer? fadeEnd;
    private (float, float) transform;

    /// <summary>The process's clock; the app sets it at launch, before any panel loads.</summary>
    internal static InkClock? Clock { get; set; }

    public static readonly DependencyProperty BackdropProperty = DependencyProperty.Register(
        nameof(Backdrop), typeof(Brush), typeof(InkPanel), new PropertyMetadata(null, (d, _) => ((InkPanel)d).surface?.Invalidate()));

    /// <summary>
    /// What the panel sits on (a solid colour brush, as a theme resource so it follows the mode):
    /// painted where there is no orb. Without one, the panel is transparent, which WinUI shows as black.
    /// </summary>
    public Brush? Backdrop
    {
        get => (Brush?)GetValue(BackdropProperty);
        set => SetValue(BackdropProperty, value);
    }

    public InkPanel()
    {
        Loaded += (_, _) => Attach();
        Unloaded += (_, _) => Detach();
        SizeChanged += (_, _) => UpdateCanvas();
        CompositionScaleChanged += (_, _) => UpdateCanvas();
        RegisterPropertyChangedCallback(VisibilityProperty, (_, _) => UpdateVisibility());
    }

    /// <summary>What the ink shows.</summary>
    public InkState State
    {
        get => state;
        set
        {
            state = value;
            if (surface is not null)
            {
                surface.State = value;
            }
        }
    }

    /// <summary>The orb's colours.</summary>
    internal GlowLook Look
    {
        get => look;
        set
        {
            look = value;
            if (surface is not null)
            {
                surface.Look = value;
            }
        }
    }

    /// <summary>The user's Always still.</summary>
    internal bool AlwaysStill
    {
        get => alwaysStill;
        set
        {
            alwaysStill = value;
            if (surface is not null)
            {
                surface.AlwaysStill = value;
            }
        }
    }

    /// <summary>
    /// The orb's opacity over its backdrop, reached over 0.8 s (OrbFade; at once without
    /// animations): dimmed while anything is live, and under High Contrast, so the text over it
    /// reads. Not the element's Opacity: that fades the panel to white by day, not to the window.
    /// A live orb draws the fade frame by frame; a still one draws once now and once at its end.
    /// </summary>
    internal float OrbOpacity
    {
        get => fade.Target;
        set
        {
            if (value == fade.Target)
            {
                return;
            }
            var animate = SystemMotion.AnimationsEnabled && !alwaysStill;
            fade.FadeTo(value, Now, animate);
            surface?.Invalidate();
            if (animate)
            {
                fadeEnd ??= DispatcherQueue.CreateTimer();
                fadeEnd.IsRepeating = false;
                fadeEnd.Interval = TimeSpan.FromSeconds(OrbFade.Duration);
                fadeEnd.Tick -= OnFadeEnd;
                fadeEnd.Tick += OnFadeEnd;
                fadeEnd.Start();
            }
        }
    }

    private static double Now => System.Diagnostics.Stopwatch.GetTimestamp() / (double)System.Diagnostics.Stopwatch.Frequency;

    private void OnFadeEnd(Microsoft.UI.Dispatching.DispatcherQueueTimer sender, object args) => surface?.Invalidate();


    /// <summary>A frame was drawn (live, or the still frame). UI thread.</summary>
    internal event Action<GlowFrame>? Drawn;

    /// <summary>Where the orb sits: the window's place unless set (the first run's orbs sit in the middle).</summary>
    internal InkPlacement Placement { get; set; } = new(GlowTokens.Orb.Main.X, GlowTokens.Orb.Main.Y, GlowTokens.Orb.Main.Unit);

    /// <summary>The prototype's stand-in voice instead of the live levels (the first run's demo). Set before it loads.</summary>
    internal bool Demo { get; set; }

    /// <summary>How far the final pass's blot goes (InkSimulation.BlotDepth): the window's orb stops partway. Set before it loads.</summary>
    internal double BlotDepth { get; set; } = 1;

    /// <summary>Frames this panel has presented (0 before it loads).</summary>
    public int FramesDrawn => surface?.FramesDrawn ?? 0;

    /// <summary>The region the orb wanders in (the main window's); null keeps it at Placement. Set before it loads.</summary>
    internal OrbWander.Bounds? WanderBounds { get; set; }

    /// <summary>At rest, on screen: to a new spot (the screen or the monitor behind it changed).</summary>
    internal void MoveAtRest() => surface?.MoveAtRest();

    /// <summary>The window was activated: a resting orb that held its spot long enough moves.</summary>
    internal void Activated() => surface?.Activated();

    /// <summary>The window was covered by others and is not any more: a resting orb moves, as on coming on screen.</summary>
    internal void Uncovered() => surface?.Uncovered();

    /// <summary>Whether it glides to a new spot now.</summary>
    internal bool IsGliding => surface?.IsGliding ?? false;

    /// <summary>The orb's centre now, as fractions of the panel (x from the left, y from the top).</summary>
    internal (double X, double Y) OrbCentre => surface?.OrbCentre ?? (Placement.X, Placement.Y);

    /// <summary>Holds now taken (OrbHold): the orb takes no new spot while any is.</summary>
    private int holds;
    private bool spotRead;

    /// <summary>
    /// Holds the orb still and returns its centre (fractions of the panel, y from the top) once it is
    /// on screen and has arrived (a glide under way, or the one it takes on coming on screen,
    /// finishes first), as the Mac's OrbHold. Each hold needs its Release, cancelled or not. Holds
    /// are counted: one that ends late never lets go of the next one's.
    /// </summary>
    internal async Task<(double X, double Y)> Hold(CancellationToken cancel)
    {
        holds++;
        if (surface is null)
        {
            return OrbCentre;
        }
        surface.HoldsStill = true;
        await surface.Settled(cancel);
        spotRead = true;
        if (surface is not null)
        {
            surface.HoldsSpot = true;
        }
        return OrbCentre;
    }

    /// <summary>Lets go of one hold; with none left the orb wanders again.</summary>
    internal void Release()
    {
        if (holds == 0)
        {
            return;
        }
        holds--;
        if (holds > 0)
        {
            return;
        }
        spotRead = false;
        if (surface is not null)
        {
            surface.HoldsSpot = false;
            surface.HoldsStill = false;
        }
    }

    private void Attach()
    {
        if (surface is not null || Clock is null)
        {
            return;
        }
        surface = new InkSurface(this, ShellInk.Loader, Clock)
        {
            State = state,
            Placement = Placement,
            Demo = Demo,
            BlotDepth = BlotDepth,
            Look = look,
            AlwaysStill = alwaysStill,
            Levels = ShellInk.LiveLevels,
            HoldsStill = holds > 0,
            HoldsSpot = spotRead,
        };
        surface.WanderBounds = WanderBounds;
        surface.Drawn += frame => Drawn?.Invoke(frame);
        root = XamlRoot;
        if (root is not null)
        {
            root.Changed += RootChanged;
        }
        UpdateCanvas();
        UpdateVisibility();
    }

    private void Detach()
    {
        if (root is not null)
        {
            root.Changed -= RootChanged;
            root = null;
        }
        surface?.Dispose();
        surface = null;
        swapChain?.Dispose();
        swapChain = null;
        transform = default;
    }

    private void RootChanged(XamlRoot sender, XamlRootChangedEventArgs args) => UpdateVisibility();

    private void UpdateVisibility() =>
        surface?.SetOnScreen(root?.IsHostVisible == true && Visibility == Visibility.Visible && ActualWidth >= 1 && ActualHeight >= 1);

    /// <summary>The prototype's sizing rule: the display scale, capped at 1.25 for a large zone and 2 otherwise.</summary>
    private void UpdateCanvas()
    {
        if (surface is null || ActualWidth < 1 || ActualHeight < 1)
        {
            UpdateVisibility();
            return;
        }
        var (w, h) = InkSurface.CanvasPixels(ActualWidth, ActualHeight, CompositionScaleX);
        surface.SetCanvas(w, h);
        UpdateVisibility();
    }

    /// <summary>The pipeline was lost or replaced: the swapchain goes with it and is made again on the next frame.</summary>
    void IInkTarget.ReleaseDeviceResources()
    {
        swapChain?.Dispose();
        swapChain = null;
        transform = default;
    }

    /// <summary>Not watched (it would tick while the window idles): its frames find failures.</summary>
    string? IInkTarget.CheckDevice() => null;

    /// <summary>The window's ink has no stand-in: the window says what failed (ShellInk).</summary>
    void IInkTarget.SetFallback(bool shown)
    {
    }

    bool IInkTarget.Render(InkPipeline pipeline, in InkUniforms uniforms)
    {
        var (w, h) = surface!.Canvas;
        if (swapChain is null)
        {
            swapChain = new CompositionSwapChain(pipeline, w, h);
            swapChain.AttachToSwapChainPanel(((WinRT.IWinRTObject)this).NativeObject.ThisPtr);
        }
        var transform = ((float)(ActualWidth / w), (float)(ActualHeight / h));
        if (swapChain.Width != w || swapChain.Height != h || transform != this.transform)
        {
            swapChain.Resize(w, h);
            // The canvas's pixels stretched over the panel's DIPs.
            swapChain.SetMatrixTransform(transform.Item1, transform.Item2);
            this.transform = transform;
        }
        if (!swapChain.DrawInk(uniforms, BackdropColour(), fade.ValueAt(Now)))
        {
            // The compositor had no room: a still frame is drawn again a frame later.
            surface!.PresentDropped();
        }
        return true;
    }

    private (float, float, float)? BackdropColour()
    {
        if (Backdrop is not SolidColorBrush brush)
        {
            return null;
        }
        // A translucent brush is taken as opaque: the panel cannot show what is behind it.
        var c = brush.Color;
        return (c.R / 255f, c.G / 255f, c.B / 255f);
    }
}
