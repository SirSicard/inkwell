// What the screen models' tests share: events from the core's JSON, and a recorder for what a
// model sent and logged (as the Mac's ScreensTests helpers).
using System.Text;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

internal static class Ev
{
    /// <summary>An event from its JSON, as the core sends it. Fails the test when this build cannot read it.</summary>
    public static InkEvent Of(string json)
    {
        var decoded = InkEvent.Decode(Encoding.UTF8.GetBytes(json));
        Assert.False(decoded is UndecodableEvent or UnknownEvent, $"not this build's event: {json}");
        return decoded;
    }

    public static T Of<T>(string json) where T : InkEvent => Assert.IsType<T>(Of(json));
}

/// <summary>What a model sent.</summary>
internal sealed class Sent
{
    public List<CoreCommand> Commands { get; } = [];

    public Action<CoreCommand> Send => Commands.Add;
}

/// <summary>What was logged.</summary>
internal sealed class Logged
{
    public List<string> Messages { get; } = [];

    public ScreenLog Log => new(Messages.Add);
}
