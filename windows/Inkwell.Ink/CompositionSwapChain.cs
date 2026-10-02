// A DXGI swapchain for composition (flip model, premultiplied alpha): what DirectComposition shows
// in the Drop's window, and what a WinUI SwapChainPanel shows in the app's. Direct3D draws into it
// through a render target view, Direct2D through a target bitmap. UI thread only.
using TerraFX.Interop.DirectX;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.DirectX.DXGI_ALPHA_MODE;
using static TerraFX.Interop.DirectX.DXGI_SCALING;
using static TerraFX.Interop.DirectX.DXGI_SWAP_EFFECT;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>A composition swapchain on the pipeline's device.</summary>
public sealed unsafe class CompositionSwapChain : IDisposable
{
    private readonly InkPipeline pipeline;
    internal IDXGISwapChain1* SwapChain;
    private ID3D11RenderTargetView* view;
    private ID2D1Bitmap1* bitmap;

    /// <summary>Its size in pixels.</summary>
    public int Width { get; private set; }
    /// <summary>Its height in pixels.</summary>
    public int Height { get; private set; }

    /// <summary>A swapchain of <paramref name="width"/> x <paramref name="height"/> pixels.</summary>
    public CompositionSwapChain(InkPipeline pipeline, int width, int height)
    {
        ArgumentNullException.ThrowIfNull(pipeline);
        this.pipeline = pipeline;
        IDXGIAdapter* adapter = null;
        IDXGIFactory2* factory = null;
        try
        {
            InkRendererException.Check(pipeline.DxgiDevice->GetAdapter(&adapter), "reach the graphics adapter");
            InkRendererException.Check(adapter->GetParent(__uuidof<IDXGIFactory2>(), (void**)&factory), "reach DXGI");
            var desc = new DXGI_SWAP_CHAIN_DESC1
            {
                Width = (uint)width,
                Height = (uint)height,
                Format = InkPipeline.PixelFormat,
                SampleDesc = new DXGI_SAMPLE_DESC { Count = 1, Quality = 0 },
                BufferUsage = DXGI.DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount = 2,
                Scaling = DXGI_SCALING_STRETCH,
                SwapEffect = DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode = DXGI_ALPHA_MODE_PREMULTIPLIED,
            };
            IDXGISwapChain1* swapChain;
            InkRendererException.Check(
                factory->CreateSwapChainForComposition((IUnknown*)pipeline.Device, &desc, null, &swapChain),
                $"make a swapchain ({width}x{height})");
            SwapChain = swapChain;
            Width = width;
            Height = height;
        }
        finally
        {
            Com.Release(ref factory);
            Com.Release(ref adapter);
        }
    }

    /// <summary>The swapchain as IUnknown, for DirectComposition's SetContent.</summary>
    internal IUnknown* Unknown => (IUnknown*)SwapChain;

    /// <summary>Resizes the buffers (a DPI or size change). Drops the views; they are made again on use.</summary>
    public void Resize(int width, int height)
    {
        if (width == Width && height == Height)
        {
            return;
        }
        ReleaseViews();
        InkRendererException.Check(SwapChain->ResizeBuffers(2, (uint)width, (uint)height, InkPipeline.PixelFormat, 0),
            $"resize the swapchain ({width}x{height})");
        Width = width;
        Height = height;
    }

    /// <summary>Direct3D's view of the back buffer (flip model: buffer 0 is always the back buffer).</summary>
    internal ID3D11RenderTargetView* RenderTargetView
    {
        get
        {
            if (view == null)
            {
                ID3D11Texture2D* buffer;
                InkRendererException.Check(SwapChain->GetBuffer(0, __uuidof<ID3D11Texture2D>(), (void**)&buffer), "reach the back buffer");
                ID3D11RenderTargetView* v;
                var hr = pipeline.Device->CreateRenderTargetView((ID3D11Resource*)buffer, null, &v);
                buffer->Release();
                InkRendererException.Check(hr, "draw into the back buffer");
                view = v;
            }
            return view;
        }
    }

    /// <summary>Direct2D's target bitmap over the back buffer.</summary>
    internal ID2D1Bitmap1* TargetBitmap
    {
        get
        {
            if (bitmap == null)
            {
                IDXGISurface* surface;
                InkRendererException.Check(SwapChain->GetBuffer(0, __uuidof<IDXGISurface>(), (void**)&surface), "reach the back buffer");
                var props = new D2D1_BITMAP_PROPERTIES1
                {
                    pixelFormat = new D2D1_PIXEL_FORMAT { format = InkPipeline.PixelFormat, alphaMode = D2D1_ALPHA_MODE.D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX = 96,
                    dpiY = 96,
                    bitmapOptions = D2D1_BITMAP_OPTIONS.D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS.D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                };
                ID2D1Bitmap1* b;
                var hr = pipeline.D2D->CreateBitmapFromDxgiSurface(surface, &props, &b);
                surface->Release();
                InkRendererException.Check(hr, "draw on the back buffer");
                bitmap = b;
            }
            return bitmap;
        }
    }

    /// <summary>
    /// Draws the orb over the whole back buffer, cleared to <paramref name="backdrop"/> (or to
    /// transparent when null), and presents it (a SwapChainPanel's frame). Over a backdrop, an
    /// <paramref name="opacity"/> under 1 fades the orb into it. UI thread.
    /// </summary>
    public void DrawInk(in InkUniforms uniforms, (float R, float G, float B)? backdrop = null, float opacity = 1)
    {
        pipeline.Encode(RenderTargetView, Width, Height, uniforms, backdrop);
        if (backdrop is { } colour && opacity < 1)
        {
            Fade(colour, opacity);
        }
        Present();
    }

    /// <summary>
    /// The backdrop over the drawn orb at 1 - <paramref name="opacity"/>: the orb at that opacity on
    /// the backdrop. A SwapChainPanel's own Opacity cannot do it: it fades to what WinUI keeps
    /// behind the panel (white by day), not to the window.
    /// </summary>
    private void Fade((float R, float G, float B) colour, float opacity)
    {
        var d2d = pipeline.D2D;
        d2d->SetTarget((ID2D1Image*)TargetBitmap);
        d2d->BeginDraw();
        ID2D1SolidColorBrush* brush = null;
        HRESULT hr;
        try
        {
            var fill = new DXGI_RGBA { r = colour.R, g = colour.G, b = colour.B, a = 1 - Math.Clamp(opacity, 0, 1) };
            InkRendererException.Check(d2d->CreateSolidColorBrush(&fill, null, &brush), "make the ink's backdrop brush");
            var all = new D2D_RECT_F { left = 0, top = 0, right = Width, bottom = Height };
            d2d->FillRectangle(&all, (ID2D1Brush*)brush);
        }
        finally
        {
            hr = d2d->EndDraw(null, null);
            d2d->SetTarget(null);
            Com.Release(ref brush);
        }
        InkRendererException.Check(hr, "fade the ink");
    }

    /// <summary>For tests: an HRESULT the next Present returns instead of presenting (a lost device), once.</summary>
    internal int InjectedPresentResult { get; set; }

    /// <summary>Presents the back buffer at the next frame. A removed or reset device throws.</summary>
    public void Present()
    {
        HRESULT hr;
        if (InjectedPresentResult != 0)
        {
            hr = InjectedPresentResult;
            InjectedPresentResult = 0;
        }
        else
        {
            hr = SwapChain->Present(1, 0);
        }
        InkRendererException.Check(hr, "present the ink");
    }

    /// <summary>
    /// Shows this swapchain in a WinUI SwapChainPanel: <paramref name="panel"/> is the panel's
    /// IUnknown (its WinRT object, AddRef'd by the caller, which keeps its own reference).
    /// </summary>
    public void AttachToSwapChainPanel(nint panel)
    {
        // ISwapChainPanelNative (microsoft.ui.xaml.media.dxinterop.h in the Windows App SDK),
        // IUnknown's three methods and then SetSwapChain.
        var iid = new Guid("63aad0b8-7c24-40ff-85a8-640d944cc325");
        void* native;
        InkRendererException.Check(((IUnknown*)panel)->QueryInterface(&iid, &native), "reach the SwapChainPanel's native interface");
        try
        {
            var setSwapChain = (delegate* unmanaged[MemberFunction]<void*, IDXGISwapChain*, int>)(*(void***)native)[3];
            InkRendererException.Check(setSwapChain(native, (IDXGISwapChain*)SwapChain), "show the ink in the window");
        }
        finally
        {
            ((IUnknown*)native)->Release();
        }
    }

    /// <summary>Scales the swapchain's pixels onto the panel's (a canvas capped below the display scale).</summary>
    public void SetMatrixTransform(float scaleX, float scaleY)
    {
        IDXGISwapChain2* two;
        InkRendererException.Check(SwapChain->QueryInterface(__uuidof<IDXGISwapChain2>(), (void**)&two), "reach IDXGISwapChain2");
        var m = new DXGI_MATRIX_3X2_F { _11 = scaleX, _22 = scaleY };
        var hr = two->SetMatrixTransform(&m);
        two->Release();
        InkRendererException.Check(hr, "scale the ink to the window");
    }

    private void ReleaseViews()
    {
        Com.Release(ref bitmap);
        Com.Release(ref view);
    }

    /// <summary>Releases the swapchain. UI thread.</summary>
    public void Dispose()
    {
        ReleaseViews();
        Com.Release(ref SwapChain);
    }
}
