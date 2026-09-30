// Against the real core: the catalogue's model.update is read as the core's own command, and its
// failure comes back with the id that names the row. The model asked for is not in the registry,
// so the core refuses it before anything is held or fetched: nothing downloads. Needs ink_ffi.dll
// (as SmokeTests), so it runs on the PC.
using Inkwell.Core.Events;
using Inkwell.Core.Native;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

[Collection(RealCore.Name)]
public class ModelUpdateCoreContractTests
{
    [Fact]
    public void AModelUpdateIsReadByTheCoreAndItsFailureNamesTheRow()
    {
        var data = Path.Combine(Path.GetTempPath(), $"inkwell-model-update-{Guid.NewGuid():N}");
        var events = new Events();
        var session = PhrasesCoreContractTests.Start(new InkConfig(data, LogLevel: "warn"), events.Record);
        try
        {
            session.Command(new CoreCommand.ModelsList().Json);
            Assert.NotNull(events.Wait<ModelsListed>(TimeSpan.FromSeconds(10)));
            var update = new CoreCommand.ModelUpdate("not-a-registry-model", "not-a-registry-model");
            session.Command(update.Json);
            var failed = events.Wait<CommandFailed>(TimeSpan.FromSeconds(10));
            Assert.NotNull(failed);
            Assert.Equal("model.update", failed.Command);
            Assert.Equal("model.update:not-a-registry-model", failed.Id);
            Assert.DoesNotContain("unknown field", failed.Message, StringComparison.Ordinal); // every field it sent is one the core reads
            Assert.DoesNotContain(events.All, e => e is ModelUpdateStarted or ModelUpdateProgress);
            Assert.DoesNotContain(events.All, e => e is UndecodableEvent or UnknownEvent);
        }
        finally
        {
            session.Shutdown();
            if (Directory.Exists(data))
            {
                Directory.Delete(data, recursive: true);
            }
        }
    }
}
