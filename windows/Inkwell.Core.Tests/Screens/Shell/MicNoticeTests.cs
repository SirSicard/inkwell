// What the store keeps, and the Drop and Live say, when the chosen mic isn't connected
// (audio.input_fallback) or a meeting's mic goes (meeting.mic_switched), as the Mac's
// MicNoticeTests. Every device name here is synthetic.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class MicNoticeTests
{
    private static readonly MicFallback Fallback = new("Headset (Buds)", "Microphone (Realtek Audio)");

    [Fact]
    public void AFallbackIsKeptForItsTakeAndGoesWithItOrWhenTheChosenMicIsBack()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"audio.input_fallback","wanted":{"id":"hs","name":"Headset (Buds)","transport":"bluetooth"},"mic_name":"Microphone (Realtek Audio)","mic_transport":"built_in"}""")]);
        Assert.Equal(Fallback, store.MicFallback);
        store.Apply([Ev.Of("""{"type":"dictation.started","take":1,"edit":false}""")]);
        Assert.NotNull(store.MicFallback); // said during the take it opened for
        store.Apply([Ev.Of("""{"type":"dictation.discarded","reason":"too_short"}""")]);
        Assert.Null(store.MicFallback); // and let go of after it

        store.Apply([Ev.Of("""{"type":"audio.input_fallback","wanted":{"id":"hs"},"mic_name":"M","mic_transport":"usb"}""")]);
        Assert.Null(store.MicFallback!.Wanted); // a choice stored without its name
        store.Apply([SoundModelTests.Devices(input: "hs", usingMic: """{"id":"mic","name":"M","transport":"usb","reason":"chosen_missing"}""", type: "audio.devices_changed")]);
        Assert.NotNull(store.MicFallback); // still missing
        store.Apply([SoundModelTests.Devices(input: "hs", usingMic: """{"id":"hs","name":"Headset (Buds)","transport":"bluetooth","reason":"chosen"}""", type: "audio.devices_changed")]);
        Assert.Null(store.MicFallback); // the chosen mic is back

        store.Apply([Ev.Of("""{"type":"audio.input_fallback","wanted":{"id":"hs"},"mic_name":"M","mic_transport":"usb"}""")]);
        store.Apply([Ev.Of("""{"type":"core.stopped"}""")]);
        Assert.Null(store.MicFallback);
    }

    /// <summary>A meeting's stand-in is said by the meeting: a take ending meanwhile keeps it, the meeting's end lets it go.</summary>
    [Fact]
    public void AFallbackSaidForAMeetingLastsAsLongAsTheMeeting()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.started","record":"r"}"""),
            Ev.Of("""{"type":"audio.input_fallback","wanted":{"id":"hs","name":"Headset (Buds)"},"mic_name":"M","mic_transport":"usb"}"""),
            Ev.Of("""{"type":"dictation.started","take":1,"edit":false}"""),
            Ev.Of("""{"type":"dictation.discarded","reason":"too_short"}"""),
        ]);
        Assert.NotNull(store.MicFallback);
        store.Apply([Ev.Of("""{"type":"meeting.finished","record":"r"}""")]);
        Assert.Null(store.MicFallback);
    }

    [Fact]
    public void AMeetingsMicThatWentIsKeptForLiveAndTheDropSaysItUntilTheNextLine()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.started","record":"r","app":"ms-teams","app_name":"Teams","mic_name":"Headset (Buds)","mic_transport":"bluetooth","mic_reason":"chosen","far_end":"everything"}"""),
            Ev.Of("""{"type":"meeting.mic_switched","record":"r","from_name":"Headset (Buds)","from_transport":"bluetooth","mic_name":"Microphone (Realtek Audio)","mic_transport":"built_in","mic_reason":"chosen_missing"}"""),
        ]);
        var meeting = store.Meeting!;
        Assert.Equal(new MicSwitch("Headset (Buds)", "Microphone (Realtek Audio)", 0), meeting.MicSwitch);
        Assert.Equal("Microphone (Realtek Audio)", meeting.MicName);
        Assert.Equal(MicReason.ChosenMissing, meeting.MicReason);
        Assert.Equal("Headset (Buds) went. Now recording with Microphone (Realtek Audio).", MeetingDrop.Live(meeting, DropInk.Meeting).Detail);
        Assert.Equal("Microphone (Realtek Audio), since Headset (Buds) went", LiveHeader.MicLine(meeting));
        // A line from either side ends it in the Drop (a listening-only meeting would never show
        // the other side's lines again otherwise); Live keeps it.
        store.Apply([Ev.Of("""{"type":"meeting.final","record":"r","channel":"far","start_ms":0,"end_ms":900,"text":"Hello"}""")]);
        Assert.Equal("Hello", MeetingDrop.Live(store.Meeting!, DropInk.Meeting).Detail);
        Assert.NotNull(store.Meeting!.MicSwitch);
    }

    [Fact]
    public void ATakeOrAMeetingOnAStandInMicSaysSo()
    {
        var listening = DictationDrop.Live(DictationPhase.Listening, new LiveDictation(1, false, null, "Notepad"), Fallback)!;
        Assert.Equal("Dictating · Notepad", listening.Title);
        Assert.Equal("Headset (Buds) isn't connected. Using Microphone (Realtek Audio).", listening.Detail);
        var words = DictationDrop.Live(DictationPhase.Listening, new LiveDictation(1, false, null, null, "hello there"), Fallback)!;
        Assert.Equal("hello there", words.Detail); // the words win
        Assert.Equal("Listening", DictationDrop.Live(DictationPhase.Listening, new LiveDictation(1, false, null, null))!.Detail);
        var waiting = MeetingDrop.Live(new LiveMeeting("r") { AppName = "Teams" }, DropInk.Meeting, Fallback);
        Assert.Equal("Headset (Buds) isn't connected. Using Microphone (Realtek Audio).", waiting.Detail);
        Assert.Equal("Your chosen mic isn't connected. Using X.", MeetingDrop.FallbackLine(new MicFallback(null, "X")));
        Assert.Equal("Your mic went. Now recording with X.", MeetingDrop.SwitchLine(new MicSwitch(null, "X", 0)));
    }

    [Fact]
    public void LivesMicLineSaysAStandIn()
    {
        var meeting = new LiveMeeting("r") { MicName = "Microphone (Realtek Audio)", MicReason = MicReason.DefaultInput };
        Assert.Null(LiveHeader.MicLine(meeting));
        Assert.Equal(
            "Microphone (Realtek Audio), until your chosen mic is back",
            LiveHeader.MicLine(meeting with { MicReason = MicReason.ChosenMissing }));
    }
}
