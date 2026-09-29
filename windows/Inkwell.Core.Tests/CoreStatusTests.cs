// What the window shows about the core: an event it cannot read or a failed command is never
// dropped silently, and nothing leaves it spinning.
using Inkwell.Core.Events;
using Xunit;

namespace Inkwell.Core.Tests;

public class CoreStatusTests
{
    private static readonly CoreStatus Starting = new(CoreStatusKind.Starting);

    [Fact]
    public void ReadyWithThisShellsAbiShowsTheVersion()
    {
        var logged = new List<string>();
        var next = Starting.Next(
            new CoreReady { Type = "core.ready", Abi = InkSession.AbiVersion, Version = "1.2.3" }, logged.Add);
        Assert.Equal(new CoreStatus(CoreStatusKind.Ready, "1.2.3"), next);
        Assert.Empty(logged);
    }

    [Fact]
    public void ReadyWithAnotherAbiFails()
    {
        var next = Starting.Next(
            new CoreReady { Type = "core.ready", Abi = InkSession.AbiVersion + 1, Version = "9" }, _ => { });
        Assert.Equal(CoreStatusKind.Failed, next.Kind);
    }

    [Theory]
    [InlineData("core.ready")]
    [InlineData("")]
    public void AnUndecodableEventStopsTheSpinnerAndIsLoggedByType(string type)
    {
        var logged = new List<string>();
        var next = Starting.Next(new UndecodableEvent { Type = type, Record = "r1" }, logged.Add);
        Assert.Equal(new CoreStatus(CoreStatusKind.MismatchedBuild, type), next);
        Assert.Contains(type.Length == 0 ? "typeless" : type, Assert.Single(logged));
        Assert.DoesNotContain("r1", logged[0]);
    }

    [Fact]
    public void AnUnknownEventIsAMismatchedBuild()
    {
        var logged = new List<string>();
        var next = new CoreStatus(CoreStatusKind.Ready, "1").Next(new UnknownEvent { Type = "no.such" }, logged.Add);
        Assert.Equal(new CoreStatus(CoreStatusKind.MismatchedBuild, "no.such"), next);
        Assert.Contains("no.such", Assert.Single(logged));
    }

    [Fact]
    public void AFailedCommandIsLoggedByNameOnly()
    {
        var logged = new List<string>();
        var ready = new CoreStatus(CoreStatusKind.Ready, "1");
        var next = ready.Next(
            new CommandFailed { Type = "command.failed", Command = "model.warm", Message = "a secret word" }, logged.Add);
        Assert.Equal(ready, next);
        Assert.Contains("model.warm", Assert.Single(logged));
        Assert.DoesNotContain("secret", logged[0]);
    }

    [Fact]
    public void StoppedIsShown()
    {
        Assert.Equal(CoreStatusKind.Stopped, Starting.Next(new CoreStopped { Type = "core.stopped" }, _ => { }).Kind);
    }
}
