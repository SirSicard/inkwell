// Glow in the shell's models: the colour rule (the same on the Mac), the appearance settings in the
// core's store, the final pass's progress, the tray's words, Owed's Undo, the automatic update
// check and Start with Windows.
using Inkwell.Core.Events;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class GlowTests
{
    [Fact]
    public void ColoursFollowThePresetTheUsersOwnAndTheModesFit()
    {
        var day = GlowScheme.Resolve(dark: false, "indigo");
        Assert.Equal("#6b5cff", day.You.Hex);
        Assert.Equal("#ffa34d", day.Them.Hex);
        Assert.Equal(GlowRgb.From(GlowTokens.Light.IdleOrb), day.Idle);
        Assert.Equal(GlowRgb.From(GlowTokens.Light.Ink), day.Ink);
        // The partner is 40 % of the way to white.
        Assert.Equal(day.You.Lift(0.4), day.YouPartner);
        // An unknown preset is the default one.
        Assert.Equal(day, GlowScheme.Resolve(dark: false, "nope"));
        // Ink & Sand's graphite is too dark for night: lifted 45 % towards white.
        var night = GlowScheme.Resolve(dark: true, "ink_sand");
        Assert.Equal(GlowRgb.FromInt(0x33384D).Lift(0.45), night.You);
        // A colour too pale for day is deepened to 70 %.
        var pale = GlowRgb.FromInt(0xF8F8F8);
        Assert.Equal(pale.Scale(0.7), GlowScheme.Resolve(dark: false, "indigo", you: pale).You);
        Assert.Equal(pale, GlowScheme.Resolve(dark: true, "indigo", you: pale).You);
        Assert.Null(GlowRgb.Parse("preset"));
        Assert.Equal("#0a0b0c", GlowRgb.Parse("#0A0B0C")!.Value.Hex);
    }

    [Fact]
    public void TheEdgeIsYoursWhileDictatingAndGoesOutWithBlotting()
    {
        var colours = GlowScheme.Resolve(dark: true, "indigo");
        Span<GlowEdgeStop> stops = stackalloc GlowEdgeStop[5];
        Assert.False(GlowEdgeGradient.Stops(0, 0, 0, 0, 0, 0, colours, stops)); // idle: nothing
        Assert.True(GlowEdgeGradient.Stops(1, 0, 0, 0.5, 0, 0, colours, stops));
        foreach (var stop in stops)
        {
            // Your two shades only, mixed: never theirs.
            Assert.InRange(stop.Colour.B, Math.Min(colours.You.B, colours.YouPartner.B) - 1e-9, 1);
        }
        Assert.False(GlowEdgeGradient.Stops(0, 1, 1, 1, 1, 3, colours, stops)); // blotted: out
    }

    [Fact]
    public void AppearanceReadsEveryKeyAndFollowsTheStore()
    {
        var sent = new Sent();
        var appearance = new AppearanceModel(sent.Send, new Logged().Log);
        appearance.Load();
        Assert.Equal(9, sent.Commands.Count(c => c is CoreCommand.SettingGet));
        Assert.Equal(AppearanceMode.System, appearance.Mode);
        Assert.True(appearance.EdgeGlow);
        Assert.True(appearance.IsDark(systemDark: true));
        appearance.Apply(Ev.Of("""{"type":"setting.value","key":"appearance.mode","value":"light"}"""));
        appearance.Apply(Ev.Of("""{"type":"setting.value","key":"appearance.dots.dark","value":"lagoon"}"""));
        appearance.Apply(Ev.Of("""{"type":"setting.value","key":"appearance.you.dark","value":"#112233"}"""));
        appearance.Apply(Ev.Of("""{"type":"setting.value","key":"appearance.edge_glow","value":"off"}"""));
        appearance.Apply(Ev.Of("""{"type":"setting.value","key":"appearance.motion","value":"still"}"""));
        Assert.False(appearance.IsDark(systemDark: true));
        Assert.Equal("lagoon", appearance.DotsDark);
        Assert.Equal("indigo", appearance.DotsLight); // each mode keeps its own
        Assert.Equal(GlowRgb.FromInt(0x112233), appearance.Custom(dark: true, you: true));
        Assert.False(appearance.EdgeGlow);
        Assert.Equal(AppearanceMotion.Still, appearance.Motion);
    }

    [Fact]
    public void APresetTakesItsModesColoursBack()
    {
        var sent = new Sent();
        var appearance = new AppearanceModel(sent.Send, new Logged().Log);
        appearance.SetCustom(dark: false, you: false, GlowRgb.FromInt(0xAABBCC));
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.AppearanceThemLight, "#aabbcc"), sent.Commands[^1]);
        appearance.SetDots(dark: false, "citrus");
        Assert.Null(appearance.Custom(dark: false, you: false));
        Assert.Contains(new CoreCommand.SettingSet(ShellSetting.AppearanceDotsLight, "citrus"), sent.Commands);
        Assert.Contains(new CoreCommand.SettingSet(ShellSetting.AppearanceThemLight, "preset"), sent.Commands);
        Assert.Throws<ArgumentException>(() => appearance.SetDots(dark: true, "nope"));
        // A save that fails is said, and the store is read again.
        appearance.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:appearance.dots.light","message":"x"}"""));
        Assert.True(appearance.Failed);
        Assert.Equal(new CoreCommand.SettingGet(ShellSetting.AppearanceDotsLight), sent.Commands[^1]);
    }

    [Fact]
    public void TheFinalPassesProgressIsKeptWhileTheMeetingBlots()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}"""), Ev.Of("""{"type":"meeting.stopped","record":"r1"}""")]);
        Assert.Null(store.Meeting!.Blotted.Words);
        store.Apply([Ev.Of("""{"type":"meeting.summarized","record":"r1","unverified":0}""")]);
        Assert.Equal("summarized", store.Meeting!.Blotted.Words);
        Assert.Equal("transcribed · speakers sorted · summarized", new BlotProgress(true, true, true).Words);
    }

    [Fact]
    public void TheTraySaysWhatIsLive()
    {
        var store = new CoreStore();
        Assert.Equal("Starting", TrayMenu.StatusLine(store, null));
        store.Apply([Ev.Of($$"""{"type":"core.ready","abi":{{InkSession.AbiVersion}},"version":"1.0.0"}""")]);
        Assert.Equal("Ready", TrayMenu.StatusLine(store, null));
        Assert.Equal((TrayMenu.RecordNowTitle, true), TrayMenu.RecordItem(store));
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1","title":"Design review"}""")]);
        Assert.Equal("Recording · Design review · 12:04", TrayMenu.StatusLine(store, 724_000));
        Assert.Equal((TrayMenu.StopTitle, true), TrayMenu.RecordItem(store));
        Assert.Equal(TrayState.Recording, TrayMenu.State(DropInk.Blotting));
        Assert.Equal(TrayState.Problem, TrayMenu.State(DropInk.Problem));
    }

    [Fact]
    public void MarkingDoneOffersUndo()
    {
        var sent = new Sent();
        var owed = new OwedModel(sent.Send, TimeZoneInfo.Utc);
        owed.Apply(Ev.Of("""{"type":"commitments.listed","items":[{"id":"a","record":"r1","record_started_at_unix_ms":0,"text":"Send the plan","merged":0}]}"""));
        owed.MarkDone("a");
        Assert.Equal("Marked done: Send the plan", owed.JustDoneLine);
        owed.UndoDone();
        Assert.Equal(new CoreCommand.CommitmentSetDone("a", false), sent.Commands[^1]);
        Assert.Null(owed.JustDone);
    }

    private sealed class Preference(bool? stored) : IUpdatePreference
    {
        public bool? Stored { get; private set; } = stored;

        public bool? Read() => Stored;

        public void Write(bool enabled) => Stored = enabled;
    }

    private sealed class Installed : IUpdater
    {
        public int Checks { get; private set; }

        public bool UpdatesItself => true;

        public Task<string?> CheckAsync()
        {
            Checks++;
            return Task.FromResult<string?>(null);
        }

        public Task DownloadAsync(Action<int> progress) => Task.CompletedTask;

        public void RestartToUpdate()
        {
        }
    }

    [Fact]
    public async Task TheAutomaticCheckRunsAtLaunchOnlyWhenOn()
    {
        var updater = new Installed();
        var preference = new Preference(null);
        var on = new UpdatesModel(updater, new Logged().Log, preference);
        Assert.True(on.AutoCheck); // on until the user turns it off
        await on.CheckAtLaunch();
        Assert.Equal(1, updater.Checks);
        Assert.Equal(UpdateState.UpToDate, on.State);
        var off = new UpdatesModel(updater, new Logged().Log, new Preference(null));
        off.SetAutoCheck(false);
        Assert.False(off.AutoCheck);
        await off.CheckAtLaunch();
        Assert.Equal(1, updater.Checks);
    }

    private sealed class Entry(string? unavailable) : IStartupEntry
    {
        public bool On { get; private set; }

        public string? Unavailable { get; } = unavailable;

        public bool IsOn() => On;

        public void Change(bool enabled) => On = enabled;
    }

    [Fact]
    public void StartWithWindowsIsTheInstalledAppsAndSaysWhyNot()
    {
        var folder = new StartupModel(new Entry(StartupModel.NotInstalledText), new Logged().Log);
        Assert.False(folder.Available);
        Assert.Equal(StartupModel.NotInstalledText, folder.Unavailable);
        folder.SetOn(true);
        Assert.False(folder.IsOn);
        var entry = new Entry(null);
        var installed = new StartupModel(entry, new Logged().Log);
        installed.SetOn(true);
        Assert.True(entry.On);
        Assert.True(installed.IsOn);
    }
}
