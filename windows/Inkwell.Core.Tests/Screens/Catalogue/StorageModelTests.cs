// Settings > Storage. The Mac has no test for its StorageModel; these check the same sums on a
// temporary folder, the failure a folder that can't be read shows, and the sizes' words.
using System.Globalization;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class StorageModelTests : IDisposable
{
    private readonly string root = Path.Combine(Path.GetTempPath(), $"inkwell-storage-{Guid.NewGuid():N}");

    public void Dispose()
    {
        if (Directory.Exists(root))
        {
            Directory.Delete(root, recursive: true);
        }
    }

    private void Write(string relative, int bytes)
    {
        var path = Path.Combine(root, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllBytes(path, new byte[bytes]);
    }

    [Fact]
    public async Task TheLibraryRecordingsAndModelsAreSummedApart()
    {
        var data = Path.Combine(root, "data");
        Write(Path.Combine("data", "library.sqlite"), 1000);
        Write(Path.Combine("data", "library.sqlite-wal"), 200);
        Write(Path.Combine("data", "meetings", "r1", "mic.flac"), 3000);
        Write(Path.Combine("data", "inkwell.lock"), 10);
        Write(Path.Combine("data", "models", "a", "weights.gguf"), 5000);
        var storage = new StorageModel(data, null, log: new Logged().Log);
        Assert.Null(storage.Sizes); // measuring
        await storage.Measure();
        Assert.False(storage.Failed);
        Assert.Equal(new StorageSizes(1200, 3000, 5000), storage.Sizes);

        // Models kept elsewhere are measured there; a folder under data named models is then a recording's.
        Write(Path.Combine("elsewhere", "b.gguf"), 700);
        var apart = new StorageModel(data, Path.Combine(root, "elsewhere"), log: new Logged().Log);
        await apart.Measure();
        Assert.Equal(new StorageSizes(1200, 8000, 700), apart.Sizes);

        // No models yet: nothing there, not a failure.
        Assert.Equal(0, StorageModel.MeasureFolders(data, Path.Combine(root, "none")).Models);
    }

    /// <summary>
    /// The Mac's 7609b53: once Settings has measured, a model that finishes installing (or fails:
    /// it may have removed what it downloaded) and a record the user deleted are measured again;
    /// before that, nobody reads the sizes and nothing is measured.
    /// </summary>
    [Fact]
    public async Task AModelInstalledOrARecordDeletedIsMeasuredAgain()
    {
        var data = Path.Combine(root, "data");
        Write(Path.Combine("data", "library.sqlite"), 1000);
        Write(Path.Combine("data", "meetings", "r1", "mic.flac"), 3000);
        var storage = new StorageModel(data, null, log: new Logged().Log);
        var installed = Ev.Of("""{"type":"model.update_finished","id":"silero-vad-v6-16k","next":"silero-vad-v6-16k","ok":true,"no_model_warm":false}""");
        await storage.Apply(installed);
        Assert.Null(storage.Sizes); // not measured before Settings asks

        await storage.Measure();
        Assert.Equal(0, storage.Sizes?.Models);
        Write(Path.Combine("data", "models", "silero", "model.onnx"), 5000);
        await storage.Apply(installed);
        Assert.Equal(5000, storage.Sizes?.Models);

        File.Delete(Path.Combine(data, "meetings", "r1", "mic.flac"));
        await storage.Apply(Ev.Of("""{"type":"record.deleted","record":"r1","kind":"meeting","audio_left":false,"scrubbed":true}"""));
        Assert.Equal(0, storage.Sizes?.Recordings);

        // Anything else measures nothing.
        Write(Path.Combine("data", "meetings", "r2", "mic.flac"), 7000);
        await storage.Apply(Ev.Of($$"""{"type":"core.ready","abi":{{InkSession.AbiVersion}},"version":"1.0.0"}"""));
        Assert.Equal(0, storage.Sizes?.Recordings);
    }

    [Fact]
    public async Task AFolderThatCantBeReadSaysSoNeverZero()
    {
        var logged = new Logged();
        var storage = new StorageModel(Path.Combine(root, "missing"), null, log: logged.Log);
        await storage.Measure();
        Assert.True(storage.Failed);
        Assert.Null(storage.Sizes);
        Assert.Equal("Couldn't measure the library's folder.", StorageModel.FailedText);
        Assert.Equal(["storage measure failed: DirectoryNotFoundException"], logged.Messages); // no path
        var none = new StorageModel(null, null);
        await none.Measure();
        Assert.True(none.Failed);
        Assert.False(none.CanReveal);
    }

    [Fact]
    public void ShowInFileExplorerHandsTheViewTheFolder()
    {
        var opened = new List<string>();
        var storage = new StorageModel(root, null, opened.Add);
        Assert.True(storage.CanReveal);
        storage.ShowInFileExplorer();
        Assert.Equal([root], opened);
    }

    [Fact]
    public void SizesReadAsWindowsWritesThem()
    {
        var invariant = CultureInfo.InvariantCulture;
        Assert.Equal("0 bytes", StorageModel.Size(0, invariant));
        Assert.Equal("512 bytes", StorageModel.Size(512, invariant));
        Assert.Equal("1.50 KB", StorageModel.Size(1536, invariant));
        Assert.Equal("1.99 KB", StorageModel.Size(2047, invariant));
        Assert.Equal("12.0 MB", StorageModel.Size(12L * 1024 * 1024, invariant));
        Assert.Equal("2.32 GB", StorageModel.Size(2_500_000_000, invariant));
        Assert.Equal("0.97 MB", StorageModel.Size(1000 * 1024, invariant));
        Assert.Equal("1,50 KB", StorageModel.Size(1536, CultureInfo.GetCultureInfo("da-DK")));
    }
}
