// The orb's one Direct3D 11 pipeline, shared by every surface that draws it (the Drop, a
// SwapChainPanel in the window) and by offscreen renders: the Mac's InkPipeline
// (mac/Sources/InkRenderer/InkPipeline.swift) on Windows. Its only resource is the uniform block
// G (160 bytes, b0); the shader writes premultiplied alpha, transparent wherever there is no orb,
// so each draw clears its target first, and blends the orb over what it cleared to: transparent
// for the Drop and offscreen renders, the colour behind the orb for a SwapChainPanel (InkPanel).
//
// The shader ships as source: Shaders/ink.hlsl, generated from shaders/ink.wgsl by
// core/crates/ink-shader, compiled here with D3DCompile (the FXC compiler in d3dcompiler_47.dll,
// part of Windows) for shader model 5.0. Direct3D 11 runs only FXC's bytecode: DXC compiles to
// DXIL, which is Direct3D 12's. Building the app needs no shader toolchain, as on the Mac. The
// compile runs once, off the UI thread (InkPipelineLoader).
//
// The device also carries Direct2D and DirectWrite (the Drop's pill and text), so it is created
// with BGRA support. Direct3D's immediate context and Direct2D's device context are
// single-threaded: everything but the constructor runs on the UI thread.
using System.Reflection;
using System.Runtime.InteropServices;
using TerraFX.Interop.DirectX;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.DirectX.D3D_DRIVER_TYPE;
using static TerraFX.Interop.DirectX.D3D11_BIND_FLAG;
using static TerraFX.Interop.DirectX.D3D11_CREATE_DEVICE_FLAG;
using static TerraFX.Interop.DirectX.D3D11_USAGE;
using static TerraFX.Interop.DirectX.DirectX;
using static TerraFX.Interop.DirectX.DXGI_FORMAT;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>Which Direct3D device draws the ink.</summary>
public enum InkAdapter
{
    /// <summary>The GPU, falling back to WARP (the software rasteriser) when there is none.</summary>
    Hardware,
    /// <summary>WARP only: tests, and machines without a usable GPU.</summary>
    Warp,
}

/// <summary>The compiled shader, its uniform buffer, the device and its Direct2D and DirectWrite factories.</summary>
public sealed unsafe class InkPipeline : IDisposable
{
    /// <summary>Every ink target's pixel format: the swapchains' and the offscreen renders'.</summary>
    public const DXGI_FORMAT PixelFormat = DXGI_FORMAT_B8G8R8A8_UNORM;

    internal ID3D11Device* Device;
    internal ID3D11DeviceContext* Context;
    internal IDXGIDevice* DxgiDevice;
    internal ID2D1Factory1* D2DFactory;
    internal ID2D1DeviceContext* D2D;
    internal IDWriteFactory* DWrite;
    private ID2D1Device* d2dDevice;
    private ID3D11VertexShader* vertexShader;
    private ID3D11PixelShader* pixelShader;
    private ID3D11Buffer* constants;
    private ID3D11RasterizerState* rasterizer;
    private ID3D11BlendState* over;
    private bool disposed;

    /// <summary>Whether this pipeline draws with WARP (no GPU, or asked for).</summary>
    public bool IsWarp { get; }

    /// <summary>The adapter's name, as DXGI reports it (for the log: which device draws the ink).</summary>
    public string AdapterName { get; private set; } = "";

    /// <summary>How long the shader took to compile.</summary>
    public TimeSpan CompileTime { get; }

    /// <summary>
    /// Creates the device and compiles the shader: <paramref name="hlsl"/>, or the embedded
    /// Shaders/ink.hlsl. Any thread. Throws <see cref="InkRendererException"/>.
    /// </summary>
    public InkPipeline(InkAdapter adapter = InkAdapter.Hardware, string? hlsl = null)
    {
        hlsl ??= ShaderSource();
        try
        {
            IsWarp = CreateDevice(adapter);
            var started = System.Diagnostics.Stopwatch.GetTimestamp();
            using var vs = Compile(hlsl, "vs_main", "vs_5_0");
            using var ps = Compile(hlsl, "fs_main", "ps_5_0");
            CompileTime = System.Diagnostics.Stopwatch.GetElapsedTime(started);
            ID3D11VertexShader* v;
            InkRendererException.Check(Device->CreateVertexShader(vs.Code, vs.Size, null, &v), "make the ink's vertex shader");
            vertexShader = v;
            ID3D11PixelShader* p;
            InkRendererException.Check(Device->CreatePixelShader(ps.Code, ps.Size, null, &p), "make the ink's pixel shader");
            pixelShader = p;

            var cb = new D3D11_BUFFER_DESC
            {
                ByteWidth = (uint)sizeof(InkUniforms),
                Usage = D3D11_USAGE_DEFAULT,
                BindFlags = (uint)D3D11_BIND_CONSTANT_BUFFER,
            };
            ID3D11Buffer* buffer;
            InkRendererException.Check(Device->CreateBuffer(&cb, null, &buffer), "make the ink's uniform buffer");
            constants = buffer;

            // The full-canvas quad is a triangle strip whose two triangles wind opposite ways:
            // nothing may be culled (Metal culls nothing by default either).
            var rd = new D3D11_RASTERIZER_DESC
            {
                FillMode = D3D11_FILL_MODE.D3D11_FILL_SOLID,
                CullMode = D3D11_CULL_MODE.D3D11_CULL_NONE,
                DepthClipEnable = true,
            };
            ID3D11RasterizerState* r;
            InkRendererException.Check(Device->CreateRasterizerState(&rd, &r), "make the ink's rasterizer state");
            rasterizer = r;

            // Premultiplied "over": the orb on what the target was cleared to. Over a transparent
            // clear it writes exactly what the shader wrote.
            var bd = new D3D11_BLEND_DESC();
            bd.RenderTarget[0] = new D3D11_RENDER_TARGET_BLEND_DESC
            {
                BlendEnable = true,
                SrcBlend = D3D11_BLEND.D3D11_BLEND_ONE,
                DestBlend = D3D11_BLEND.D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp = D3D11_BLEND_OP.D3D11_BLEND_OP_ADD,
                SrcBlendAlpha = D3D11_BLEND.D3D11_BLEND_ONE,
                DestBlendAlpha = D3D11_BLEND.D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha = D3D11_BLEND_OP.D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask = (byte)D3D11_COLOR_WRITE_ENABLE.D3D11_COLOR_WRITE_ENABLE_ALL,
            };
            ID3D11BlendState* b;
            InkRendererException.Check(Device->CreateBlendState(&bd, &b), "make the ink's blend state");
            over = b;
        }
        catch
        {
            Dispose();
            throw;
        }
    }

    /// <summary>Whether the device is gone (removed, reset, hung): only a new pipeline draws again. UI thread.</summary>
    internal bool DeviceRemoved() => !disposed && Device != null && Device->GetDeviceRemovedReason().FAILED;

    /// <summary>The HRESULTs that mean the device itself is lost, not just a host's objects.</summary>
    internal static bool IsDeviceLoss(int hr) =>
        hr == DXGI.DXGI_ERROR_DEVICE_REMOVED || hr == DXGI.DXGI_ERROR_DEVICE_RESET || hr == DXGI.DXGI_ERROR_DEVICE_HUNG
        || hr == DXGI.DXGI_ERROR_DRIVER_INTERNAL_ERROR || hr == D2DERR.D2DERR_RECREATE_TARGET;

    /// <summary>Shaders/ink.hlsl, embedded in this assembly.</summary>
    public static string ShaderSource()
    {
        using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("ink.hlsl")
            ?? throw new InkRendererException("couldn't find the ink shader (ink.hlsl) in the app") { Permanent = true };
        using var reader = new StreamReader(stream);
        return reader.ReadToEnd();
    }

    private bool CreateDevice(InkAdapter adapter)
    {
        var flags = (uint)D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        var levels = stackalloc D3D_FEATURE_LEVEL[] { D3D_FEATURE_LEVEL.D3D_FEATURE_LEVEL_11_0 };
        ID3D11Device* device = null;
        ID3D11DeviceContext* context = null;
        var warp = adapter == InkAdapter.Warp;
        HRESULT hr = E.E_FAIL;
        if (!warp)
        {
            hr = D3D11CreateDevice(null, D3D_DRIVER_TYPE_HARDWARE, HMODULE.NULL, flags, levels, 1, D3D11.D3D11_SDK_VERSION, &device, null, &context);
            warp = hr.FAILED;
        }
        if (warp)
        {
            hr = D3D11CreateDevice(null, D3D_DRIVER_TYPE_WARP, HMODULE.NULL, flags, levels, 1, D3D11.D3D11_SDK_VERSION, &device, null, &context);
        }
        InkRendererException.Check(hr, "make a Direct3D 11 device");
        Device = device;
        Context = context;

        IDXGIDevice* dxgi;
        InkRendererException.Check(Device->QueryInterface(__uuidof<IDXGIDevice>(), (void**)&dxgi), "reach the DXGI device");
        DxgiDevice = dxgi;
        IDXGIAdapter* dxgiAdapter;
        if (DxgiDevice->GetAdapter(&dxgiAdapter).SUCCEEDED)
        {
            DXGI_ADAPTER_DESC desc;
            if (dxgiAdapter->GetDesc(&desc).SUCCEEDED)
            {
                AdapterName = new string(&desc.Description.e0);
            }
            dxgiAdapter->Release();
        }

        var options = new D2D1_FACTORY_OPTIONS { debugLevel = D2D1_DEBUG_LEVEL.D2D1_DEBUG_LEVEL_NONE };
        ID2D1Factory1* factory;
        InkRendererException.Check(
            D2D1CreateFactory(D2D1_FACTORY_TYPE.D2D1_FACTORY_TYPE_SINGLE_THREADED, __uuidof<ID2D1Factory1>(), &options, (void**)&factory),
            "make a Direct2D factory");
        D2DFactory = factory;
        ID2D1Device* d2d;
        InkRendererException.Check(D2DFactory->CreateDevice(DxgiDevice, &d2d), "make a Direct2D device");
        d2dDevice = d2d;
        ID2D1DeviceContext* dc;
        InkRendererException.Check(d2dDevice->CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS.D2D1_DEVICE_CONTEXT_OPTIONS_NONE, &dc), "make a Direct2D device context");
        D2D = dc;

        IDWriteFactory* dwrite;
        InkRendererException.Check(
            DWriteCreateFactory(DWRITE_FACTORY_TYPE.DWRITE_FACTORY_TYPE_SHARED, __uuidof<IDWriteFactory>(), (IUnknown**)&dwrite),
            "make a DirectWrite factory");
        DWrite = dwrite;
        return warp;
    }

    /// <summary>A compiled shader stage.</summary>
    private readonly struct Blob(ID3DBlob* blob) : IDisposable
    {
        public void* Code => blob->GetBufferPointer();
        public nuint Size => blob->GetBufferSize();
        public void Dispose() => blob->Release();
    }

    private static Blob Compile(string hlsl, string entry, string target)
    {
        var source = System.Text.Encoding.UTF8.GetBytes(hlsl);
        var entryBytes = System.Text.Encoding.ASCII.GetBytes(entry + "\0");
        var targetBytes = System.Text.Encoding.ASCII.GetBytes(target + "\0");
        var name = "ink.hlsl\0"u8;
        ID3DBlob* code = null;
        ID3DBlob* errors = null;
        HRESULT hr;
        fixed (byte* src = source)
        fixed (byte* e = entryBytes)
        fixed (byte* t = targetBytes)
        fixed (byte* n = name)
        {
            hr = D3DCompile(src, (nuint)source.Length, (sbyte*)n, null, null, (sbyte*)e, (sbyte*)t,
                D3DCOMPILE.D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &code, &errors);
        }
        string? message = null;
        if (errors != null)
        {
            message = Marshal.PtrToStringAnsi((nint)errors->GetBufferPointer(), (int)errors->GetBufferSize())?.TrimEnd('\0', '\n', '\r');
            errors->Release();
        }
        if (hr.FAILED || code == null)
        {
            if (code != null)
            {
                code->Release();
            }
            throw new InkRendererException($"couldn't compile the ink shader ({entry}, {target}): {message ?? $"0x{hr.Value:X8}"}") { Permanent = true };
        }
        return new Blob(code);
    }

    /// <summary>
    /// Clears <paramref name="target"/> (<paramref name="width"/> x <paramref name="height"/>
    /// pixels) to transparent and draws the orb over it, premultiplied. UI thread. Allocation-free.
    /// </summary>
    public void Encode(ID3D11RenderTargetView* target, int width, int height, in InkUniforms uniforms) =>
        Encode(target, width, height, uniforms, null);

    /// <summary>
    /// Clears <paramref name="target"/> to <paramref name="backdrop"/> (opaque, 0 to 1), or to
    /// transparent when null, and draws the orb over it, premultiplied. UI thread. Allocation-free.
    /// </summary>
    public void Encode(ID3D11RenderTargetView* target, int width, int height, in InkUniforms uniforms, (float R, float G, float B)? backdrop)
    {
        ObjectDisposedException.ThrowIf(disposed, this);
        var ctx = Context;
        // Transparent: the orb is drawn over whatever is behind its surface. A backdrop: the
        // surface is opaque and shows that colour where there is no orb.
        var clear = stackalloc float[4] { 0, 0, 0, 0 };
        if (backdrop is var (r, g, b))
        {
            clear[0] = r;
            clear[1] = g;
            clear[2] = b;
            clear[3] = 1;
        }
        ctx->ClearRenderTargetView(target, clear);
        fixed (InkUniforms* u = &uniforms)
        {
            ctx->UpdateSubresource((ID3D11Resource*)constants, 0, null, u, 0, 0);
        }
        var viewport = new D3D11_VIEWPORT { TopLeftX = 0, TopLeftY = 0, Width = width, Height = height, MinDepth = 0, MaxDepth = 1 };
        ctx->OMSetRenderTargets(1, &target, null);
        ctx->RSSetViewports(1, &viewport);
        ctx->RSSetState(rasterizer);
        ctx->OMSetBlendState(over, null, 0xFFFFFFFF);
        ctx->IASetInputLayout(null);
        ctx->IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY.D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        ctx->VSSetShader(vertexShader, null, 0);
        ctx->PSSetShader(pixelShader, null, 0);
        var cb = constants;
        ctx->PSSetConstantBuffers(0, 1, &cb);
        ctx->Draw(4, 0);
        // Unbind the target: Direct2D or a copy may use the texture next.
        ID3D11RenderTargetView* none = null;
        ctx->OMSetRenderTargets(1, &none, null);
    }

    /// <summary>Releases every Direct3D, Direct2D and DirectWrite object. UI thread.</summary>
    public void Dispose()
    {
        if (disposed)
        {
            return;
        }
        disposed = true;
        Com.Release(ref over);
        Com.Release(ref rasterizer);
        Com.Release(ref constants);
        Com.Release(ref pixelShader);
        Com.Release(ref vertexShader);
        Com.Release(ref DWrite);
        Com.Release(ref D2D);
        Com.Release(ref d2dDevice);
        Com.Release(ref D2DFactory);
        Com.Release(ref DxgiDevice);
        if (Context != null)
        {
            Context->ClearState();
        }
        Com.Release(ref Context);
        Com.Release(ref Device);
    }
}

/// <summary>Copies a texture to the CPU through a staging texture.</summary>
internal static unsafe class Readback
{
    public static byte[] Copy(InkPipeline pipeline, ID3D11Resource* source, int width, int height, DXGI_FORMAT format, int bytesPerPixel)
    {
        var desc = new D3D11_TEXTURE2D_DESC
        {
            Width = (uint)width,
            Height = (uint)height,
            MipLevels = 1,
            ArraySize = 1,
            Format = format,
            SampleDesc = new DXGI_SAMPLE_DESC { Count = 1, Quality = 0 },
            Usage = D3D11_USAGE_STAGING,
            CPUAccessFlags = (uint)D3D11_CPU_ACCESS_FLAG.D3D11_CPU_ACCESS_READ,
        };
        ID3D11Texture2D* staging;
        InkRendererException.Check(pipeline.Device->CreateTexture2D(&desc, null, &staging), $"make a readback texture ({width}x{height})");
        try
        {
            pipeline.Context->CopyResource((ID3D11Resource*)staging, source);
            D3D11_MAPPED_SUBRESOURCE mapped;
            InkRendererException.Check(
                pipeline.Context->Map((ID3D11Resource*)staging, 0, D3D11_MAP.D3D11_MAP_READ, 0, &mapped),
                "read the frame back");
            var rowBytes = width * bytesPerPixel;
            var bytes = new byte[rowBytes * height];
            for (var y = 0; y < height; y++)
            {
                new ReadOnlySpan<byte>((byte*)mapped.pData + (nint)y * mapped.RowPitch, rowBytes).CopyTo(bytes.AsSpan(y * rowBytes));
            }
            pipeline.Context->Unmap((ID3D11Resource*)staging, 0);
            return bytes;
        }
        finally
        {
            staging->Release();
        }
    }
}
