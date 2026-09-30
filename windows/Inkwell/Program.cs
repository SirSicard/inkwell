// The app's entry point, in place of the one WinUI generates (Inkwell.csproj defines
// DISABLE_XAML_GENERATED_MAIN): Velopack's hooks run first, then WinUI starts through the
// generated XamlGeneratedMain, as the generated Main would start it.
//
// Velopack's installer and updater start the app with a hook argument (--veloapp-install,
// -updated, -obsolete, -uninstall) and wait for it: VelopackApp answers those here and exits
// before any window, doing nothing else. Uninstalling removes the install folder only; the
// library is elsewhere (DataLocation), so it stays until the user deletes it. A normal start
// passes through.
using Velopack;

namespace Inkwell;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        VelopackApp.Build().Run();
        XamlGeneratedProgram.XamlGeneratedMain();
    }
}
