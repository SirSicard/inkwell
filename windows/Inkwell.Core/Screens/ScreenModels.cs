// Every screen's view model, fed by the core's events after the CoreStore, and sending their
// commands through one delegate (the controller's Send), as the Mac's ScreenModels. One instance
// per app. Kept here, headless, so the wiring is tested: the apply order, the loads when the core
// is ready, and which failures a screen shows (the rest are logged by name).
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class ScreenModels
{
    private readonly ScreenLog log;

    /// <param name="send">Where the screens' commands go.</param>
    /// <param name="dataDirectory">The library's folder (Storage), or null when it could not be found.</param>
    /// <param name="modelsDirectory">The models' folder when it is not under the library's.</param>
    /// <param name="reveal">Opens a folder in File Explorer (Storage).</param>
    /// <param name="apps">Names the apps modes are for.</param>
    /// <param name="wake">Up next's one-shot wake (the view's clock).</param>
    public ScreenModels(
        Action<CoreCommand> send,
        string? dataDirectory = null,
        string? modelsDirectory = null,
        Action<string>? reveal = null,
        IAppDirectory? apps = null,
        IWakeScheduler? wake = null,
        ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.log = log ?? ScreenLog.System;
        // No calendar in this unpackaged build (Calendar.cs): every screen says so. Packaging (S3.6)
        // can pass Windows' calendar here.
        var cal = NoCalendar.Instance;
        Permissions = new PermissionsModel(send, cal);
        Polish = new PolishModel(send);
        Catalogue = new CatalogueModel(send);
        Modes = new ModesModel(send, apps);
        Owed = new OwedModel(send);
        Live = new LiveModel(send, log: this.log);
        Meetings = new MeetingModel(send, cal, log: this.log);
        Onboarding = new OnboardingModel(send, this.log);
        Storage = new StorageModel(dataDirectory, modelsDirectory, reveal, this.log);
        Dictation = new DictationModel(send);
        EditConsent = AiSettings.NewEditConsent(send);
        MeetingsConsent = AiSettings.NewMeetingsConsent(send);
        Ai = new AiSettings(Polish, Dictation, EditConsent, MeetingsConsent, send);
        Snippets = new SnippetsModel(send);
        VoiceCommands = new VoiceCommandsModel(send);
        ImportNote = new ImportNoteModel(send);
        RecordControls = new RecordControlsModel();
        UpNext = new UpNextModel(cal, cal, wake ?? NoWake.Instance);
    }

    public PermissionsModel Permissions { get; }
    public PolishModel Polish { get; }
    public CatalogueModel Catalogue { get; }
    public ModesModel Modes { get; }
    public OwedModel Owed { get; }
    public LiveModel Live { get; }
    public MeetingModel Meetings { get; }
    public OnboardingModel Onboarding { get; }
    public StorageModel Storage { get; }
    public DictationModel Dictation { get; }
    /// <summary>Voice edit's consent (the key is the dictation model's).</summary>
    public ConsentModel EditConsent { get; }
    /// <summary>The consent and switch for a meeting's summary and Ask (Settings > AI).</summary>
    public ConsentModel MeetingsConsent { get; }
    /// <summary>Settings > AI: the three switches and the voice-edit key's consent.</summary>
    public AiSettings Ai { get; }
    public SnippetsModel Snippets { get; }
    public VoiceCommandsModel VoiceCommands { get; }
    public ImportNoteModel ImportNote { get; }
    /// <summary>The foot of Today's ink zone.</summary>
    public RecordControlsModel RecordControls { get; }
    public UpNextModel UpNext { get; }

    /// <summary>A batch of the core's events, after the CoreStore has applied it.</summary>
    public void Apply(IReadOnlyList<InkEvent> batch)
    {
        ArgumentNullException.ThrowIfNull(batch);
        foreach (var e in batch)
        {
            if (e is CoreReady)
            {
                CoreReady();
            }
            Permissions.Apply(e);
            Polish.Apply(e);
            Catalogue.Apply(e);
            Modes.Apply(e);
            Owed.Apply(e);
            Live.Apply(e);
            Meetings.Apply(e);
            Onboarding.Apply(e);
            Dictation.Apply(e);
            EditConsent.Apply(e);
            MeetingsConsent.Apply(e);
            Snippets.Apply(e);
            VoiceCommands.Apply(e);
            ImportNote.Apply(e);
            RecordControls.Apply(e);
        }
    }

    /// <summary>
    /// The core started: read what the first screens need. The permission check is the one after
    /// each launch (Today's banner and Settings read it too).
    /// </summary>
    private void CoreReady()
    {
        Onboarding.Load();
        Polish.Load();
        Meetings.Load();
        Permissions.Refresh();
        Catalogue.Requery();
        // Reads the switch, then (unless it is off) the core holds the keys.
        Dictation.Load();
        EditConsent.Load();
        MeetingsConsent.Load();
    }

    /// <summary>The app came to the front again.</summary>
    public void AppBecameActive()
    {
        Permissions.AppBecameActive();
        Dictation.AppBecameActive();
    }

    /// <summary>The app is quitting: the first run is not skipped by it.</summary>
    public void AppQuitting() => Onboarding.AppQuitting();

    /// <summary>The core is about to stop: hand it what the screens hold unsaved.</summary>
    public void FlushBeforeStop() => Live.NotesLeft();

    /// <summary>Whether a screen shows this failure itself (the rest are logged).</summary>
    public bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return PermissionsModel.Handles(failed) || Polish.Handles(failed) || CatalogueModel.Handles(failed)
            || ModesModel.Handles(failed) || OwedModel.Handles(failed) || LiveModel.Handles(failed)
            || MeetingModel.Handles(failed) || OnboardingModel.Handles(failed) || DictationModel.Handles(failed)
            || Ai.Handles(failed) || SnippetsModel.Handles(failed) || VoiceCommandsModel.Handles(failed);
    }

    /// <summary>
    /// Logs, by command name only, each failure in <paramref name="batch"/> that no screen shows
    /// (the core's message and the command's fields are never repeated: they can hold the user's words).
    /// </summary>
    public void LogUnshown(IReadOnlyList<InkEvent> batch)
    {
        ArgumentNullException.ThrowIfNull(batch);
        foreach (var failed in batch.OfType<CommandFailed>())
        {
            if (!Handles(failed))
            {
                log.Write($"command.failed for a {failed.Command} command; no screen shows it");
            }
        }
    }

    /// <summary>A wake that never comes (tests, and until the window gives Up next its clock).</summary>
    private sealed class NoWake : IWakeScheduler
    {
        public static NoWake Instance { get; } = new();

        public IDisposable After(TimeSpan delay, Action wake) => new Nothing();

        private sealed class Nothing : IDisposable
        {
            public void Dispose()
            {
            }
        }
    }
}
