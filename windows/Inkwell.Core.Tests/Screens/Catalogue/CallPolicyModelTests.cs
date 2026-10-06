// Per-app call recording in Settings > Meetings, as the Mac's CallPolicySettingsTests: the default
// for apps not chosen for (Always, Ask or Never) with its warning and hint, each app the core has
// seen with its own choice, never by its executable's name, and a stored list the core cannot read,
// started over only after the user agrees.
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class CallPolicyModelTests
{
    private const string Zoom = "zoom.exe";

    private const string List =
        """{"app":"zoom.exe","app_name":"zoom","policy":"always","chosen":true,"seen_unix_ms":1759658400000},"""
        + """{"app":"examplecall.exe","app_name":"examplecall","policy":"ask","chosen":false},"""
        + """{"app":"contoso.callapp_8wekyb3d8bbwe!app","app_name":"Contoso Call","policy":"ask","chosen":false},"""
        + """{"app":"other.exe","app_name":"an app","policy":"ask","chosen":false}""";

    private static InkEvent Calls(string apps, string policy = "ask", string? message = null, string? @ref = null) =>
        Ev.Of($$"""{"type":"meetings.calls","default":"{{policy}}","apps":[{{apps}}]{{(message is null ? "" : $",\"message\":\"{message}\"")}}{{(@ref is null ? "" : $",\"ref\":\"{@ref}\"")}}}""");

    [Fact]
    public void TheListAndTheDefaultComeFromTheCore()
    {
        var sent = new Sent();
        var calls = new CallPolicyModel(sent.Send);
        calls.Load();
        Assert.Equal([new CoreCommand.MeetingsCallsList("calls:1")], sent.Commands);
        Assert.Null(calls.Default); // not known until the core answers
        calls.Apply(Calls(List, @ref: "calls:1"));
        Assert.Equal(CallPolicy.Ask, calls.Default);
        // Never an executable's name: well known, its stem, or the name detection saw.
        Assert.Equal(["Zoom", "Examplecall", "Contoso Call", "Other"], calls.Rows.Select(r => r.Label.Name));
        Assert.Equal([CallChoice.Always, CallChoice.Default, CallChoice.Default, CallChoice.Default], calls.Rows.Select(r => r.Choice));
        Assert.Equal("Default (Ask)", calls.Title(CallChoice.Default));
        Assert.Equal(CallPolicy.Always, calls.PolicyOf(Zoom));
        Assert.Null(calls.PolicyOf("unknown.exe"));
    }

    [Fact]
    public void AChoiceIsSentShownAndSettledByTheAnswer()
    {
        var sent = new Sent();
        var calls = new CallPolicyModel(sent.Send);
        calls.Apply(Calls(List));
        calls.Choose(CallChoice.Never, "examplecall.exe", CallPolicyOrigin.Settings);
        Assert.Equal(new CoreCommand.MeetingsCallsSet("examplecall.exe", "never", false, "calls:1"), sent.Commands[^1]);
        Assert.Equal(CallChoice.Never, calls.Rows[1].Choice); // shown as made until the answer
        Assert.Equal(CallPolicy.Never, calls.PolicyOf("examplecall.exe"));
        calls.Choose(CallChoice.Default, Zoom, CallPolicyOrigin.Settings);
        Assert.Equal(new CoreCommand.MeetingsCallsSet(Zoom, "default", false, "calls:2"), sent.Commands[^1]);
        calls.Apply(Calls(
            """{"app":"zoom.exe","app_name":"zoom","policy":"ask","chosen":false},{"app":"examplecall.exe","policy":"never","chosen":true}""",
            @ref: "calls:2"));
        Assert.Equal([CallChoice.Default, CallChoice.Never], calls.Rows.Select(r => r.Choice));
        // A failure is said, and the list read again.
        calls.Choose(CallChoice.Always, Zoom, CallPolicyOrigin.Settings);
        calls.Apply(Ev.Of("""{"type":"command.failed","command":"meetings.calls.set","id":"calls:3","message":"database is locked"}"""));
        Assert.Equal("Couldn't save that: database is locked", calls.Failure);
        Assert.Equal(CallChoice.Default, calls.Rows[0].Choice);
        Assert.Equal(new CoreCommand.MeetingsCallsList("calls:4"), sent.Commands[^1]);
    }

    [Fact]
    public void TheDefaultIsASettingWithAWarningUnderAlwaysAndAHintUnderNever()
    {
        var sent = new Sent();
        var calls = new CallPolicyModel(sent.Send);
        calls.Apply(Calls(""));
        calls.SetDefault(CallPolicy.Always);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.MeetingsCallsDefault, "always"), sent.Commands[^1]);
        Assert.Equal(
            """{"cmd":"setting.set","id":"setting:meetings.calls.default","key":"meetings.calls.default","value":"always"}""",
            new CoreCommand.SettingSet(ShellSetting.MeetingsCallsDefault, "always").Json);
        Assert.Equal(CallPolicy.Always, calls.Default);
        calls.Apply(Ev.Of("""{"type":"setting.value","key":"meetings.calls.default","value":"never"}"""));
        Assert.Equal(CallPolicy.Never, calls.Default);
        Assert.Contains("without asking", CallPolicyModel.AlwaysWarning, StringComparison.Ordinal);
        Assert.Contains("Tell the people on the call", CallPolicyModel.AlwaysWarning, StringComparison.Ordinal);
        Assert.Contains("asks instead", CallPolicyModel.AlwaysWarning, StringComparison.Ordinal);
        Assert.Contains("Record now", CallPolicyModel.NeverHint, StringComparison.Ordinal);
        foreach (var words in new[] { CallPolicyModel.AlwaysWarning, CallPolicyModel.NeverHint, CallPolicyModel.DefaultCaption })
        {
            foreach (var word in new[] { "invisible", "undetectable", "hidden", "secret", "Mac" })
            {
                Assert.DoesNotContain(word, words, StringComparison.Ordinal);
            }
        }
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:meetings.calls.default","message":"database is locked"}""");
        calls.Apply(failed);
        Assert.Equal("Couldn't save the default: database is locked", calls.Failure);
        Assert.Equal(new CoreCommand.SettingGet(ShellSetting.MeetingsCallsDefault), sent.Commands[^1]);
        Assert.True(CallPolicyModel.Handles(failed));
    }

    [Fact]
    public void AnUnreadableListStartsOverOnlyAfterTheUserAgrees()
    {
        var sent = new Sent();
        var calls = new CallPolicyModel(sent.Send);
        calls.Apply(Calls("", message: "the stored choices cannot be read"));
        Assert.Equal("the stored choices cannot be read", calls.Unreadable);
        calls.Choose(CallChoice.Never, Zoom, CallPolicyOrigin.Settings);
        Assert.Empty(sent.Commands); // nothing sent before the user agrees
        Assert.NotNull(calls.StartingOver);
        calls.CancelStartOver();
        Assert.Null(calls.StartingOver);
        calls.Choose(CallChoice.Never, Zoom, CallPolicyOrigin.Settings);
        calls.ConfirmStartOver(calls.StartingOver!.Value);
        Assert.Equal([new CoreCommand.MeetingsCallsSet(Zoom, "never", true, "calls:1")], sent.Commands);
        calls.Apply(Calls(
            """{"app":"zoom.exe","policy":"never","chosen":true}""",
            message: "the stored choices could not be read and were started over; the default is Ask now", @ref: "calls:1"));
        Assert.Null(calls.Unreadable); // readable again
        Assert.Equal("the stored choices could not be read and were started over; the default is Ask now", calls.Note);
        // From the Drop there is no room to ask: it says where to choose.
        calls.Apply(Calls("", message: "unreadable again"));
        calls.Choose(CallChoice.Always, Zoom, CallPolicyOrigin.Drop);
        Assert.Equal(CallPolicyModel.UnreadableFromDrop, calls.DropFailure);
        Assert.Contains("unreadable again", CallPolicyModel.UnreadableLine("unreadable again"), StringComparison.Ordinal);
    }

    [Fact]
    public void ACoreThatStoppedLeavesNoChoiceInFlight()
    {
        var sent = new Sent();
        var calls = new CallPolicyModel(sent.Send);
        calls.Apply(Calls(List));
        calls.Apply(Ev.Of("""{"type":"meeting.detected","app":"zoom.exe","app_name":"zoom"}"""));
        var saved = false;
        calls.Choose(CallChoice.Never, Zoom, CallPolicyOrigin.Settings, () => saved = true);
        Assert.Equal(CallChoice.Never, calls.Rows[0].Choice);
        calls.Apply(Ev.Of("""{"type":"core.stopped"}"""));
        Assert.Equal(CallChoice.Always, calls.Rows[0].Choice); // the stored choice
        calls.Apply(Calls(List, @ref: "calls:1"));
        Assert.False(saved); // a late answer runs nothing
    }

    [Fact]
    public void TheLastCallCaption()
    {
        var now = new DateTimeOffset(2026, 10, 6, 10, 0, 0, TimeSpan.Zero);
        Assert.Equal("Last call today", CallPolicyModel.SeenCaption(now.AddMinutes(-5), now, CultureInfo.InvariantCulture));
        Assert.Null(CallPolicyModel.SeenCaption(null, now, CultureInfo.InvariantCulture));
        Assert.Equal("Last call 3 Oct", CallPolicyModel.SeenCaption(now.AddDays(-3), now, CultureInfo.InvariantCulture));
        Assert.Equal("Last call 3 Oct 2025", CallPolicyModel.SeenCaption(now.AddDays(-368), now, CultureInfo.InvariantCulture));
    }
}
