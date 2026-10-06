// Every screen's view model, fed by the core's events after the CoreStore, and sending their
// commands through one delegate (the controller's Send), as the Mac's ScreenModels. One instance
// per app. Kept here, headless, so the wiring is tested: the apply order, the loads when the core
// is ready, and which failures a screen shows (the rest are logged by name).
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class ScreenModels
{
    private readonly ScreenLog log;
    private readonly Action<CoreCommand> send;

    /// <param name="send">Where the screens' commands go.</param>
    /// <param name="dataDirectory">The library's folder (Storage), or null when it could not be found.</param>
    /// <param name="modelsDirectory">The models' folder when it is not under the library's.</param>
    /// <param name="reveal">Opens a folder in File Explorer (Storage).</param>
    /// <param name="apps">Names the apps modes are for.</param>
    /// <param name="runningApps">The apps with a window now (the mode editor's Running now).</param>
    /// <param name="wake">Up next's one-shot wake (the view's clock).</param>
    /// <param name="makePlayer">Makes a record's player over the app's audio output (null: no player).</param>
    /// <param name="search">Waits for typing to pause before a Library search (null: at once).</param>
    /// <param name="appVersion">The app's version for About (null: a development build).</param>
    /// <param name="updater">General's updater (null: this copy does not update itself).</param>
    /// <param name="updatePreference">Where "Check for updates automatically" is kept (null: not offered).</param>
    /// <param name="startup">Where Start with Windows is set (null: this copy cannot start with Windows).</param>
    public ScreenModels(
        Action<CoreCommand> send,
        string? dataDirectory = null,
        string? modelsDirectory = null,
        Action<string>? reveal = null,
        IAppDirectory? apps = null,
        IWakeScheduler? wake = null,
        Func<RecordDocument, RecordPlayer?>? makePlayer = null,
        ISearchScheduler? search = null,
        string? appVersion = null,
        ScreenLog? log = null,
        IUpdater? updater = null,
        IUpdatePreference? updatePreference = null,
        IStartupEntry? startup = null,
        IRunningApps? runningApps = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.log = log ?? ScreenLog.System;
        // No calendar in this unpackaged build (Calendar.cs): every screen says so. Packaging (S3.6)
        // can pass Windows' calendar here.
        var cal = NoCalendar.Instance;
        Permissions = new PermissionsModel(send, cal);
        Polish = new PolishModel(send);
        // Models go under the library's folder unless kept elsewhere: that volume's free space.
        Catalogue = new CatalogueModel(send) { Volume = CatalogueModel.VolumeOf(modelsDirectory ?? dataDirectory) };
        // A mode's own OK is one of polish's consents; its chip reads polish's switch.
        Modes = new ModesModel(send, apps, runningApps, Polish.Consent, () => Polish.Preference);
        Owed = new OwedModel(send);
        Live = new LiveModel(send, log: this.log);
        Meetings = new MeetingModel(send, cal, log: this.log, wake: wake);
        Calls = new CallPolicyModel(send, apps);
        Live.Discarding = record => Meetings.Discarding == record;
        Import02 = new Import02Model(send, this.log);
        Onboarding = new OnboardingModel(send, this.log, Import02);
        Storage = new StorageModel(dataDirectory, modelsDirectory, reveal, this.log);
        Dictation = new DictationModel(send);
        EditConsent = AiSettings.NewEditConsent(send);
        MeetingsConsent = AiSettings.NewMeetingsConsent(send);
        Ai = new AiSettings(Polish, Dictation, EditConsent, MeetingsConsent, send);
        Recorder = new ShortcutRecorderModel(send, Dictation, token => Ai.ChooseEditKey(token), wake ?? NoWake.Instance);
        Cloud = new CloudModel(send);
        Snippets = new SnippetsModel(send);
        VoiceCommands = new VoiceCommandsModel(send);
        ImportNote = new ImportNoteModel(send);
        RecordControls = new RecordControlsModel();
        UpNext = new UpNextModel(cal, cal, wake ?? NoWake.Instance);
        Library = new LibraryModel(send, makePlayer, search);
        About = new AboutModel(appVersion);
        Updates = new UpdatesModel(updater ?? NoUpdater.Instance, this.log, updatePreference);
        Startup = new StartupModel(startup, this.log);
        Appearance = new AppearanceModel(send, this.log);
        Stats = new StatsModel(send, wake ?? NoWake.Instance);
        Sound = new SoundModel(send);
        Onboarding.Wake = wake ?? NoWake.Instance;
        Onboarding.HasLiveWords = () => Catalogue.Line(Job.LivePartials) is { Known: true, Engine: not null };
    }

    public PermissionsModel Permissions { get; }
    public PolishModel Polish { get; }

    /// <summary>"Record a shortcut…" for both keys (Settings > Dictation).</summary>
    public ShortcutRecorderModel Recorder { get; }
    public CatalogueModel Catalogue { get; }
    public ModesModel Modes { get; }
    public OwedModel Owed { get; }
    public LiveModel Live { get; }
    public MeetingModel Meetings { get; }
    /// <summary>Each app's call policy, and the default (Settings > Meetings, the Drop's offer).</summary>
    public CallPolicyModel Calls { get; }
    public OnboardingModel Onboarding { get; }
    public StorageModel Storage { get; }
    public DictationModel Dictation { get; }
    /// <summary>Voice edit's consent (the key is the dictation model's).</summary>
    public ConsentModel EditConsent { get; }
    /// <summary>The consent and switch for a meeting's summary and Ask (Settings > AI).</summary>
    public ConsentModel MeetingsConsent { get; }
    /// <summary>Settings > AI: the three switches and the voice-edit key's consent.</summary>
    public AiSettings Ai { get; }
    /// <summary>Settings > AI's language model: an own-key provider (Windows has none on the device).</summary>
    public CloudModel Cloud { get; }
    public SnippetsModel Snippets { get; }
    public VoiceCommandsModel VoiceCommands { get; }
    public ImportNoteModel ImportNote { get; }
    /// <summary>Inkwell 0.2's data: the first run's step and a row in Settings > Voice.</summary>
    public Import02Model Import02 { get; }
    /// <summary>Today's hero: whether Inkwell listens, the dictation key, Record now.</summary>
    public RecordControlsModel RecordControls { get; }
    public UpNextModel UpNext { get; }
    /// <summary>What Today, the Library and a record show of the library.</summary>
    public LibraryModel Library { get; }
    public AboutModel About { get; }

    /// <summary>General's updates row, and the tray's Check for Updates….</summary>
    public UpdatesModel Updates { get; }

    /// <summary>Start with Windows (General, and the tray).</summary>
    public StartupModel Startup { get; }

    /// <summary>Glow's mode, dots, colours, edge glow and motion (Settings > Appearance, and every surface).</summary>
    public AppearanceModel Appearance { get; }

    /// <summary>The Stats screen, milestones, and Settings > Stats.</summary>
    public StatsModel Stats { get; }

    /// <summary>Settings > Sound: the microphone, the output Record now records, and the mic test.</summary>
    public SoundModel Sound { get; }

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
            Calls.Apply(e);
            Onboarding.Apply(e);
            Import02.Apply(e);
            if (Onboarding.Showing)
            {
                // The first run offers its import step only when there is something to import.
                Import02.CheckOnce();
            }
            if (e is ImportFinished)
            {
                // What became of 0.2's key, and the key it set.
                ImportNote.Load();
                send(new CoreCommand.SettingGet(ShellSetting.DictationKey));
                // The lists it brought, which Settings may show already: an edit to the old list
                // would save it over the import's (the user's own list wins in the core).
                Snippets.Load();
                VoiceCommands.Load();
                Modes.Load();
            }
            Dictation.Apply(e);
            Recorder.Apply(e);
            EditConsent.Apply(e);
            MeetingsConsent.Apply(e);
            Cloud.Apply(e);
            Snippets.Apply(e);
            VoiceCommands.Apply(e);
            ImportNote.Apply(e);
            RecordControls.Apply(e);
            Appearance.Apply(e);
            Stats.Apply(e);
            Sound.Apply(e);
            // The sizes a model install or a deleted record changed; the view shows them when they land.
            _ = Storage.Apply(e);
        }
        // The Library folds a batch at once, and refreshes once per batch.
        Library.Apply(batch);
        // As the Mac's controller, once the screens have read what they need: the dictation model
        // is kept warm from the start, so the first dictation after a launch is not a cold load.
        // Only when this batch leaves the core ready with this shell's ABI (the Mac's store status:
        // another ABI fails, and a core.stopped after it stops).
        if (batch.LastOrDefault(e => e is Events.CoreReady or CoreStopped) is Events.CoreReady { Abi: InkSession.AbiVersion })
        {
            send(new CoreCommand.ModelWarm(Job.DictationFinal));
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
        Calls.Load();
        // Meetings a crash interrupted are finished now (the Mac waits for its own engines first;
        // this shell registers none).
        Meetings.Recover();
        Permissions.Refresh();
        Catalogue.Requery();
        // Reads the switch, then (unless it is off) the core holds the keys.
        Dictation.Load();
        EditConsent.Load();
        MeetingsConsent.Load();
        // Whether an own-key provider is ready decides whether the AI switches can be used.
        Cloud.Load();
        Appearance.Load();
    }

    /// <summary>
    /// A button on the Drop. Always for and Never for set the app's call policy; the rest are the
    /// meeting commands'. "Always for" records the call once Always is saved, while it is still
    /// offered (an app offered and made Always stays offered until it is started: inkwell.h,
    /// meetings.calls.set); a save that failed records nothing and says so on the offer.
    /// </summary>
    public void PerformDropAction(DropAction action)
    {
        ArgumentNullException.ThrowIfNull(action);
        switch (action)
        {
            case DropAction.Always always:
                Calls.Choose(CallChoice.Always, always.App, CallPolicyOrigin.Drop, () => Meetings.Record(always.App));
                break;
            case DropAction.Never never:
                // The core withdraws the offer (meeting.detection_ended, dismissed).
                Calls.Choose(CallChoice.Never, never.App, CallPolicyOrigin.Drop);
                break;
            default:
                Meetings.Perform(action);
                break;
        }
    }

    /// <summary>What a failed Drop button says on the Drop: the meeting command's, else the call policy's.</summary>
    public string? DropFailure => Meetings.FailureOn(MeetingPlace.Drop) ?? Calls.DropFailure;

    /// <summary>The app came to the front again.</summary>
    public void AppBecameActive()
    {
        Permissions.AppBecameActive();
        Dictation.AppBecameActive();
        // The calendar may have changed while the app was away (Today re-reads it, as on the Mac).
        UpNext.Refresh();
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
            || MeetingModel.Handles(failed) || CallPolicyModel.Handles(failed) || OnboardingModel.Handles(failed) || DictationModel.Handles(failed) || ShortcutRecorderModel.Handles(failed)
            || Ai.Handles(failed) || CloudModel.Handles(failed) || SnippetsModel.Handles(failed) || VoiceCommandsModel.Handles(failed)
            || Library.Handles(failed) || Import02Model.Handles(failed) || AppearanceModel.Handles(failed) || StatsModel.Handles(failed)
            || SoundModel.Handles(failed);
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
