// Each route's screen, made once from the screens' models (ScreenModels), and the first-run sheet
// the window holds over them. Navigation between screens goes through the Router; opening a record
// goes to the Library with the record asked for.
using Inkwell.Core;
using Inkwell.Core.Screens;
using Inkwell.Screens;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace Inkwell;

internal sealed class AppScreens(CoreStore store, ScreenModels models, Router router, GlowTheme theme, ShellInk ink)
{
    /// <summary>Whether the window is on screen (the window updates it): Up next's minute redraws only then.</summary>
    public WindowPresence Presence { get; } = new();

    /// <summary>The screens' models over the controller, with the app's own services.</summary>
    public static ScreenModels Models(CoreController core, DispatcherQueue ui, IUpdater? updater = null)
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
        var screens = new ScreenModels(
            core.Send,
            dataDirectory: data,
            modelsDirectory: modelsDir,
            reveal: FileExplorer.Reveal,
            apps: InstalledApps.Shared,
            wake: new DispatcherWake(ui),
            makePlayer: document => WindowsAudioOutput.PlayerFor(document, core.CommandLog),
            search: new DispatcherSearchScheduler(ui),
            appVersion: AppVersion.Release,
            log: core.CommandLog,
            updater: updater,
            // "Check for updates automatically" beside the library, as the terms' record is.
            updatePreference: data is null ? null : new UpdatePreferenceFile(Path.Combine(data, UpdatePreferenceFile.FileName)),
            startup: new WindowsStartup());
        // A moved library (development, tests, scripts) never looks at this PC's Inkwell 0.2 data.
        screens.Import02.Looks = !DataLocation.IsMoved();
        return screens;
    }

    /// <summary>A route's screen.</summary>
    public UIElement Screen(Route route) => route switch
    {
        Route.Today => new TodayScreen(
            store, models.Library, models.Owed, models.Permissions, models.UpNext, Presence, router.Open, OpenRecord,
            models.RecordControls, models.Meetings, models.Live, models.Catalogue),
        Route.Library => new LibraryScreen(models.Library, () => models.Ai.SummaryOffNote, Presence),
        Route.Owed => new OwedScreen(models.Owed, (record, ms) => OpenRecord(record, ms, play: true)),
        Route.Live => new LiveScreen(store, models.Live, models.Meetings, Presence, models.Catalogue),
        Route.Settings => new SettingsScreen(SettingsSections()),
        _ => throw new ArgumentOutOfRangeException(nameof(route)),
    };

    /// <summary>"Search everything said": the Library with the matches.</summary>
    public void Search(string query)
    {
        router.Open(Route.Library);
        models.Library.Query = query;
    }

    /// <summary>The first-run sheet, over the window while it is not completed.</summary>
    public void AttachFirstRun(FrameworkElement? host)
    {
        if (host is not null)
        {
            OnboardingSheet.Attach(
                host, models.Onboarding, models.Permissions, models.Polish, models.Cloud, models.Dictation, models.Catalogue,
                models.Import02, models.ImportNote, theme, ink);
        }
    }

    /// <summary>Opens <paramref name="record"/> in the Library, at <paramref name="ms"/> when given, playing when asked.</summary>
    public void OpenRecord(string record, long? ms, bool play)
    {
        router.Open(Route.Library);
        models.Library.Open(record, ms, play);
    }

    /// <summary>Settings' sections, in the plan's order.</summary>
    private List<SettingsSectionEntry> SettingsSections()
    {
        var importNote = new ImportKeyNoteView(models.ImportNote, () => DictationModel.Key(models.Dictation.CurrentKey)?.Name ?? DictationModel.Cap(models.Dictation.CurrentKey));
        // Inkwell 0.2's history, in General, looked for each time Settings shows it.
        var import02 = new Import02Card(models.Import02, inSettings: true);
        import02.Loaded += (_, _) => models.Import02.Check();
        return
        [
            new("General", new GeneralSection(models.Startup, models.Updates, import02)),
            new("Appearance", new AppearanceSection(theme)),
            new("Permissions", new PermissionsSection(models.Permissions)),
            new("Dictation", new VoiceSection(models.Ai, models.Recorder, importNote)),
            new("Modes", new ModesSection(models.Modes)),
            new("Snippets", new SnippetsSection(models.Snippets)),
            new("Voice commands", new VoiceCommandsSection(models.VoiceCommands)),
            new("AI", new AiSection(models.Ai, models.Cloud)),
            new("Meetings", new MeetingsSection(models.Meetings)),
            new("Models", new ModelsSection(models.Catalogue)),
            new("Storage", new StorageSection(models.Storage, models.Meetings)),
            new("About", new AboutSection(models.About)),
        ];
    }
}
