// Why the ink cannot draw. Every failure has a name and says what failed; the surface then shows
// plain paper (or, for the Drop, its text on paper) and the failure is logged by that name.
using TerraFX.Interop.Windows;

namespace Inkwell.Ink;

/// <summary>A Direct3D, DXGI, DirectComposition, Direct2D or DirectWrite call failed, or the shader did not compile.</summary>
public sealed class InkRendererException : Exception
{
    /// <summary>The failing HRESULT, or 0 when the failure has none (a shader compile error).</summary>
    public int HResultCode { get; }

    /// <summary>A failure with a message only.</summary>
    public InkRendererException(string message)
        : base(message)
    {
    }

    /// <summary>A failure with a message and an inner exception.</summary>
    public InkRendererException(string message, Exception inner)
        : base(message, inner)
    {
    }

    /// <summary>A failure without detail (for serializers and analyzers).</summary>
    public InkRendererException()
    {
    }

    internal InkRendererException(string what, HRESULT hr)
        : base($"couldn't {what} (0x{hr.Value:X8})")
    {
        HResultCode = hr.Value;
    }

    /// <summary>Throws when <paramref name="hr"/> failed; <paramref name="what"/> reads after "couldn't".</summary>
    internal static void Check(HRESULT hr, string what)
    {
        if (hr.FAILED)
        {
            throw new InkRendererException(what, hr);
        }
    }
}

/// <summary>COM pointer helpers.</summary>
internal static unsafe class Com
{
    /// <summary>Releases <paramref name="p"/> if set and clears it.</summary>
    public static void Release<T>(ref T* p)
        where T : unmanaged, IUnknown.Interface
    {
        if (p != null)
        {
            p->Release();
            p = null;
        }
    }
}
