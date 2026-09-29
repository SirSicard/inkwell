// The ink in the app's window: a SwapChainPanel holding a composition swapchain that Direct3D 11
// draws the ink into, on the shared clock while live and once per change otherwise (InkSurface).
// The Mac's InkZone. The screens place it (the rail, Today's zone with the wordmark).
//
// It counts as on screen while its window is shown (XamlRoot.IsHostVisible), it is visible and it
// has a size: hidden to the tray, the window's ink draws nothing.
using Inkwell.Ink;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell;

// The surface and the swapchain are released on Unloaded (Detach); a XAML element is never disposed.
[System.Diagnostics.CodeAnalysis.SuppressMessage("Reliability", "CA1001", Justification = "Released on Unloaded")]
public sealed partial class InkPanel : SwapChainPanel, IInkTarget
{
    private InkSurface? surface;
    private CompositionSwapChain? swapChain;
    private XamlRoot? root;
    private InkState state = InkState.Idle;
    private bool wordmark;
    private (float, float) transform;

    /// <summary>The process's clock; the app sets it at launch, before any panel loads.</summary>
    internal static InkClock? Clock { get; set; }

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

    /// <summary>Whether INKWELL is knocked out of the ink (Today's zone).</summary>
    public bool ShowsWordmark
    {
        get => wordmark;
        set
        {
            wordmark = value;
            if (surface is not null)
            {
                surface.ShowsWordmark = value;
            }
        }
    }

    /// <summary>Frames this panel has presented (0 before it loads).</summary>
    public int FramesDrawn => surface?.FramesDrawn ?? 0;

    private void Attach()
    {
        if (surface is not null || Clock is null)
        {
            return;
        }
        surface = new InkSurface(this, ShellInk.Loader, Clock) { State = state, ShowsWordmark = wordmark };
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
        surface.SetCanvas(w, h, ActualWidth);
        UpdateVisibility();
    }

    bool IInkTarget.Render(InkPipeline pipeline, in InkUniforms uniforms, InkMark? mark)
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
        swapChain.DrawInk(uniforms, mark);
        return true;
    }
}
