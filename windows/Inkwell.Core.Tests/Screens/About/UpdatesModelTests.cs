// Settings > About's updates row over a fake updater: what it says and offers at each step, that
// nothing runs until pressed, and that a failure says "Couldn't ..." with its reason and offers the
// check again.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class UpdatesModelTests
{
    private sealed class FakeUpdater(bool updatesItself = true) : IUpdater
    {
        public bool UpdatesItself { get; } = updatesItself;
        public string? Offered { get; set; } = "1.0.1";
        public Exception? CheckFails { get; set; }
        public Exception? DownloadFails { get; set; }
        public int Checks { get; private set; }
        public int Downloads { get; private set; }
        public int Restarts { get; private set; }

        public Task<string?> CheckAsync()
        {
            Checks++;
            return CheckFails is null ? Task.FromResult(Offered) : Task.FromException<string?>(CheckFails);
        }

        public Task DownloadAsync(Action<int> progress)
        {
            Downloads++;
            progress(40);
            return DownloadFails is null ? Task.CompletedTask : Task.FromException(DownloadFails);
        }

        public void RestartToUpdate() => Restarts++;
    }

    [Fact]
    public async Task AFolderBuildSaysUpdatesAreForTheInstalledAppAndOffersNothing()
    {
        var updater = new FakeUpdater(updatesItself: false);
        var updates = new UpdatesModel(updater, new Logged().Log);
        Assert.Equal(UpdateState.Off, updates.State);
        Assert.Equal("Updates work only in the installed app.", updates.Line);
        Assert.Null(updates.ActionTitle);
        Assert.False(updates.CanAct);
        await updates.Act();
        Assert.Equal(0, updater.Checks);
    }

    [Fact]
    public async Task NothingIsCheckedUntilPressedThenCheckDownloadAndRestart()
    {
        var updater = new FakeUpdater();
        var updates = new UpdatesModel(updater, new Logged().Log);
        Assert.Equal(UpdateState.Idle, updates.State);
        Assert.Null(updates.Line);
        Assert.Equal("Check Now", updates.ActionTitle);
        Assert.True(updates.CanAct);
        Assert.Equal(0, updater.Checks);

        await updates.Act();
        Assert.Equal(1, updater.Checks);
        Assert.Equal(UpdateState.Available, updates.State);
        Assert.Equal("Inkwell 1.0.1 is available.", updates.Line);
        Assert.Equal("Download and Install", updates.ActionTitle);

        await updates.Act();
        Assert.Equal(1, updater.Downloads);
        Assert.Equal(UpdateState.Ready, updates.State);
        Assert.Equal("Inkwell 1.0.1 is downloaded. Inkwell restarts to install it.", updates.Line);
        Assert.Equal("Restart to Update", updates.ActionTitle);
        Assert.Equal(0, updater.Restarts);

        await updates.Act();
        Assert.Equal(1, updater.Restarts);
    }

    [Fact]
    public async Task UpToDateSaysSoAndChecksAgainWhenPressed()
    {
        var updater = new FakeUpdater { Offered = null };
        var updates = new UpdatesModel(updater, new Logged().Log);
        await updates.Act();
        Assert.Equal(UpdateState.UpToDate, updates.State);
        Assert.Equal("Inkwell is up to date.", updates.Line);
        Assert.Equal("Check Now", updates.ActionTitle);
        await updates.Act();
        Assert.Equal(2, updater.Checks);
    }

    [Fact]
    public async Task AFailedCheckSaysCouldntWithItsReasonAndOffersTheCheckAgain()
    {
        var logged = new Logged();
        var updater = new FakeUpdater { CheckFails = new HttpRequestException("Response status code does not indicate success: 403 (rate limit exceeded)") };
        var updates = new UpdatesModel(updater, logged.Log);
        await updates.Act();
        Assert.Equal(UpdateState.Failed, updates.State);
        Assert.Equal("Couldn't check for updates: Response status code does not indicate success: 403 (rate limit exceeded).", updates.Line);
        Assert.Equal("Check Now", updates.ActionTitle);
        Assert.True(updates.CanAct);
        // The log names the step, not the reason.
        Assert.Equal(["update check failed"], logged.Messages);

        updater.CheckFails = null;
        await updates.Act();
        Assert.Equal(UpdateState.Available, updates.State);
    }

    [Fact]
    public async Task AFailedDownloadSaysCouldntAndNeverOffersTheRestart()
    {
        var updater = new FakeUpdater { DownloadFails = new InvalidOperationException("SHA256 doesn't match") };
        var updates = new UpdatesModel(updater, new Logged().Log);
        await updates.Act();
        await updates.Act();
        Assert.Equal(UpdateState.Failed, updates.State);
        Assert.Equal("Couldn't download Inkwell 1.0.1: SHA256 doesn't match.", updates.Line);
        Assert.Equal("Check Now", updates.ActionTitle);
        await updates.Act();
        Assert.Equal(2, updater.Checks);
        Assert.Equal(0, updater.Restarts);
    }

    [Fact]
    public async Task ThereIsNoSecondCheckWhileOneRuns()
    {
        var gate = new TaskCompletionSource<string?>();
        var updater = new SlowUpdater(gate.Task);
        var updates = new UpdatesModel(updater, new Logged().Log);
        var first = updates.Act();
        Assert.Equal(UpdateState.Checking, updates.State);
        Assert.Equal("Checking for updates…", updates.Line);
        Assert.False(updates.CanAct);
        await updates.Act();
        Assert.Equal(1, updater.Checks);
        gate.SetResult(null);
        await first;
        Assert.Equal(UpdateState.UpToDate, updates.State);
    }

    private sealed class SlowUpdater(Task<string?> answer) : IUpdater
    {
        public bool UpdatesItself => true;
        public int Checks { get; private set; }

        public Task<string?> CheckAsync()
        {
            Checks++;
            return answer;
        }

        public Task DownloadAsync(Action<int> progress) => Task.CompletedTask;

        public void RestartToUpdate()
        {
        }
    }
}
