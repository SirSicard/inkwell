// One frame of the orb, offscreen, at a fixed time: what the tests check. No window, no clock, so
// it runs over SSH and on CI (with WARP).
//
// The recipe: a zeroed state at time t, one step on the snap path (the weights at the state's,
// the voice the prototype's synthetic one), drawn moving at that time.
using TerraFX.Interop.DirectX;
using static TerraFX.Interop.DirectX.D3D11_BIND_FLAG;
using static TerraFX.Interop.DirectX.D3D11_USAGE;

namespace Inkwell.Ink;

/// <summary>A rendered frame: 8-bit RGBA, premultiplied, row 0 at the top; (0, 0, 0, 0) where there is no orb.</summary>
public sealed class InkImage(int width, int height, byte[] rgba, InkUniforms uniforms)
{
    /// <summary>Width in pixels.</summary>
    public int Width { get; } = width;
    /// <summary>Height in pixels.</summary>
    public int Height { get; } = height;
    /// <summary>RGBA bytes.</summary>
    public byte[] Rgba { get; } = rgba;
    /// <summary>The uniforms it was drawn with.</summary>
    public InkUniforms Uniforms { get; } = uniforms;

    /// <summary>The pixel at (x, y) from the top left.</summary>
    public (byte R, byte G, byte B) Pixel(int x, int y)
    {
        var i = (y * Width + x) * 4;
        return (Rgba[i], Rgba[i + 1], Rgba[i + 2]);
    }

    /// <summary>The pixel's alpha at (x, y).</summary>
    public byte Alpha(int x, int y) => Rgba[(y * Width + x) * 4 + 3];

    /// <summary>Rec. 601 luminance, 0..255.</summary>
    public double Luminance(int x, int y)
    {
        var (r, g, b) = Pixel(x, y);
        return 0.299 * r + 0.587 * g + 0.114 * b;
    }
}

/// <summary>Offscreen frames.</summary>
public static unsafe class InkSnapshot
{
    /// <summary>
    /// Draws <paramref name="state"/> at time <paramref name="t"/> into a canvas of the given
    /// pixels, with <paramref name="look"/>'s colours (default: day, Indigo &amp; Coral) and the orb
    /// at <paramref name="placement"/> (default: the centre). UI thread (or any one thread that
    /// owns the pipeline).
    /// </summary>
    public static InkImage Render(InkPipeline pipeline, InkState state, double t, int width, int height,
        InkVoice? voice = null, GlowLook? look = null, InkPlacement? placement = null)
    {
        ArgumentNullException.ThrowIfNull(pipeline);
        if (width < 2 || height < 2)
        {
            throw new InkRendererException($"couldn't render a {width}x{height} canvas");
        }
        var simulation = new InkSimulation(InkRandom.Seeded(1))
        {
            State = state,
            CanvasWidth = width,
            CanvasHeight = height,
        };
        simulation.ApplyFixed(t, voice ?? InkVoice.Synthetic);
        var uniforms = simulation.Uniforms(placement ?? InkPlacement.Centre, look ?? GlowLook.Default, moving: true);

        using var target = new InkTexture(pipeline, width, height);
        pipeline.Encode(target.View, width, height, uniforms);
        var bgra = Readback.Copy(pipeline, (ID3D11Resource*)target.Texture, width, height, InkPipeline.PixelFormat, 4);
        // BGRA to RGBA; the alpha stays (premultiplied).
        for (var i = 0; i < bgra.Length; i += 4)
        {
            (bgra[i], bgra[i + 2]) = (bgra[i + 2], bgra[i]);
        }
        return new InkImage(width, height, bgra, uniforms);
    }
}

/// <summary>An offscreen BGRA target the ink draws into and Direct2D can read (the Drop's orb, snapshots).</summary>
internal sealed unsafe class InkTexture : IDisposable
{
    internal ID3D11Texture2D* Texture;
    internal ID3D11RenderTargetView* View;

    public int Width { get; }
    public int Height { get; }

    public InkTexture(InkPipeline pipeline, int width, int height)
    {
        Width = width;
        Height = height;
        var desc = new D3D11_TEXTURE2D_DESC
        {
            Width = (uint)width,
            Height = (uint)height,
            MipLevels = 1,
            ArraySize = 1,
            Format = InkPipeline.PixelFormat,
            SampleDesc = new DXGI_SAMPLE_DESC { Count = 1, Quality = 0 },
            Usage = D3D11_USAGE_DEFAULT,
            BindFlags = (uint)(D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE),
        };
        ID3D11Texture2D* texture;
        InkRendererException.Check(pipeline.Device->CreateTexture2D(&desc, null, &texture), $"make an ink canvas ({width}x{height})");
        Texture = texture;
        ID3D11RenderTargetView* view;
        var hr = pipeline.Device->CreateRenderTargetView((ID3D11Resource*)texture, null, &view);
        if (hr.FAILED)
        {
            Dispose();
            InkRendererException.Check(hr, "draw into the ink canvas");
        }
        View = view;
    }

    public void Dispose()
    {
        Com.Release(ref View);
        Com.Release(ref Texture);
    }
}
