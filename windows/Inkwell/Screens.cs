// Each route's screen, made once from the screens' models (ScreenModels), and the pieces the
// window holds beside them: the foot of Today's ink zone and the first-run sheet. Navigation between
// screens goes through the Router; opening a record goes to the Library with the record asked for.
using Inkwell.Core;
using Inkwell.Core.Screens;
using Inkwell.Screens;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell;

internal sealed class AppScreens(CoreStore store, ScreenModels models, Router router)
{
    /// <summary>The screens' models over the controller, with the app's own services.</summary>
    public static ScreenModels Models(CoreController core, DispatcherQueue ui)
    {
        string? data = null;
        string? modelsDir = null;
        try
        {
            data = DataLocation.DataDirectory();
            modelsDir = DataLocation.ModelsDirectory();
        }
        catch (IOException)
        {
            // The controller shows why the core did not start; Storage says it couldn't measure.
        }
        // The installed apps are indexed off the UI thread, before Settings > Modes asks.
        InstalledApps.Shared.Warm();
        return new ScreenModels(
            core.Send,
            dataDirectory: data,
            modelsDirectory: modelsDir,
            reveal: FileExplorer.Reveal,
            apps: InstalledApps.Shared,
            log: core.CommandLog);
    }

    /// <summary>A route's screen.</summary>
    public UIElement Screen(Route route) => route switch
    {
        Route.Owed => new OwedScreen(models.Owed, (record, ms) => OpenRecord(record, ms, play: true)),
        Route.Live => new LiveScreen(store, models.Live, models.Meetings),
        Route.Settings => new SettingsScreen(SettingsSections()),
        _ => Placeholder(route),
    };

    /// <summary>The foot of Today's ink zone.</summary>
    public static UIElement InkZoneFoot() => new StackPanel();

    /// <summary>The first-run sheet, over the window while it is not completed.</summary>
    public void AttachFirstRun(FrameworkElement? host)
    {
        if (host is not null)
        {
            OnboardingSheet.Attach(host, models.Onboarding, models.Permissions, models.Polish, models.Dictation);
        }
    }

    /// <summary>Opens <paramref name="record"/> in the Library, at <paramref name="ms"/> when given, playing when asked.</summary>
    public void OpenRecord(string record, long? ms, bool play)
    {
        router.Open(Route.Library);
        models.Library.Open(record, ms, play);
    }

    /// <summary>Settings' sections, in the canvas's order.</summary>
    private List<SettingsSectionEntry> SettingsSections()
    {
        var importNote = new ImportKeyNoteView(models.ImportNote, () => KeyNames.Display(models.Dictation.CurrentKey));
        return
        [
            new("Permissions", new PermissionsSection(models.Permissions)),
            new("Voice", new VoiceSection(models.Ai, importNote)),
            new("AI", new AiSection(models.Ai)),
            new("Modes", new ModesSection(models.Modes)),
            new("Snippets", new SnippetsSection(models.Snippets)),
            new("Voice commands", new VoiceCommandsSection(models.VoiceCommands)),
            new("Meetings", new MeetingsSection(models.Meetings)),
            new("Models", new ModelsSection(models.Catalogue)),
            new("Storage", new StorageSection(models.Storage, models.Meetings)),
        ];
    }

    private static StackPanel Placeholder(Route route)
    {
        var title = new TextBlock { Text = route.Title(), Style = (Style)Application.Current.Resources["InkScreenTitleStyle"] };
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level1);
        return new StackPanel { Padding = new Thickness(48, 38, 48, 28), Children = { title } };
    }
}
