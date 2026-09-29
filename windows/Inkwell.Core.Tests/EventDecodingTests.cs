// The generated event types (Generated/Events.g.cs) read what the core sends, and keep what they
// cannot read as UnknownEvent or UndecodableEvent instead of failing, as the Swift types do.
using System.Text;
using System.Text.Json;
using Inkwell.Core.Events;
using Xunit;

namespace Inkwell.Core.Tests;

public class EventDecodingTests
{
    private static InkEvent Decode(string json) => InkEvent.Decode(Encoding.UTF8.GetBytes(json));

    [Fact]
    public void AKnownEventDecodesToItsRecord()
    {
        var ready = Assert.IsType<CoreReady>(Decode("""{"type":"core.ready","abi":2,"version":"0.0.0"}"""));
        Assert.Equal("core.ready", ready.Type);
        Assert.Equal(2, ready.Abi);
        Assert.Equal("0.0.0", ready.Version);
    }

    [Fact]
    public void EnumsReadTheirJsonStrings()
    {
        var state = Assert.IsType<MeetingSideState>(Decode(
            """{"type":"meeting.side_state","record":"r1","channel":"far","state":"ok"}"""));
        Assert.Equal(Channel.Far, state.Channel);
    }

    [Fact]
    public void AnUnknownTypeIsKeptAsUnknown()
    {
        var unknown = Assert.IsType<UnknownEvent>(Decode("""{"type":"no.such","x":1}"""));
        Assert.Equal("no.such", unknown.Type);
    }

    [Theory]
    // A value this build does not know, a missing required field, a wrong type, a number for an
    // enum, and null for a field that is never null.
    [InlineData("""{"type":"meeting.side_state","record":"r1","channel":"left","state":"ok"}""")]
    [InlineData("""{"type":"meeting.side_state","record":"r1","state":"ok"}""")]
    [InlineData("""{"type":"meeting.side_state","record":"r1","channel":"far","state":1}""")]
    [InlineData("""{"type":"meeting.side_state","record":"r1","channel":0,"state":"ok"}""")]
    [InlineData("""{"type":"meeting.side_state","record":"r1","channel":null,"state":"ok"}""")]
    public void AKnownEventThatDoesNotDecodeKeepsItsTypeAndRecord(string json)
    {
        var bad = Assert.IsType<UndecodableEvent>(Decode(json));
        Assert.Equal("r1", bad.Record);
        Assert.StartsWith("meeting.", bad.Type);
    }

    [Theory]
    [InlineData("""{"abi":2}""")]
    [InlineData("""{"type":7}""")]
    [InlineData("[]")]
    public void OnlyJsonWithoutAStringTypeThrows(string json)
    {
        Assert.ThrowsAny<JsonException>(() => Decode(json));
    }
}
