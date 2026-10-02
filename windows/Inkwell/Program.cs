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
//
// One app per library: a second start (Start with Windows and a click, say) hands its activation
// to the running one, which shows its window, and exits before it starts a core. Two ran side by
// side before, each with its own core on the one library, its own tray icon and its own dictation
// key hook, so a dictation was typed twice. The key is the library's folder, so a library moved
// with INK_DATA_DIR runs beside the user's.
using Microsoft.Windows.AppLifecycle;
using Velopack;

namespace Inkwell;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        VelopackApp.Build().SetAutoApplyOnStartup(false).Run();
        if (!IsTheLibrarysApp())
        {
            return;
        }
        XamlGeneratedProgram.XamlGeneratedMain();
    }

    /// <summary>
    /// Whether this process is the library's app: true when none was running (this one is now), or
    /// when the library's folder is not known (the core then says why). Otherwise the activation
    /// went to the one that runs.
    /// </summary>
    private static bool IsTheLibrarysApp()
    {
        string key;
        try
        {
            key = "library:" + Path.GetFullPath(DataLocation.DataDirectory()).TrimEnd('\\').ToUpperInvariant();
        }
        catch (IOException)
        {
            return true;
        }
        var owner = AppInstance.FindOrRegisterForKey(key);
        if (owner.IsCurrent)
        {
            return true;
        }
        // Off this STA thread, whose message loop has not started: the redirection is a COM call.
        var args = AppInstance.GetCurrent().GetActivatedEventArgs();
        Task.Run(() => owner.RedirectActivationToAsync(args).AsTask()).Wait();
        return false;
    }
}
