// The app's entry point, in place of the one WinUI generates (Inkwell.csproj defines
// DISABLE_XAML_GENERATED_MAIN): Velopack's hooks run first, then WinUI starts through the
// generated XamlGeneratedMain, as the generated Main would start it.
//
// Velopack's installer and updater start the app with a hook argument (--veloapp-install,
// -updated, -obsolete, -uninstall) and wait for it: VelopackApp answers those here and exits
// before any window, doing nothing else. Uninstalling removes the install folder only; the
// library is elsewhere (DataLocation), so it stays until the user deletes it. A normal start
// passes through: Velopack's own default would install an update that was downloaded but not yet
// restarted into, at the next start, and restart; auto-apply is off, so an update installs only
// when the user presses Restart to Update (UpdatesModel).
using Velopack;

namespace Inkwell;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        VelopackApp.Build().SetAutoApplyOnStartup(false).Run();
        XamlGeneratedProgram.XamlGeneratedMain();
    }
}
