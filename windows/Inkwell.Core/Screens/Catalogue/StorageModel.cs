// Settings > Storage: where the library lives, and how much room each part takes. As the Mac's
// StorageModel (ScreenModels.swift).
//
// Sizes are measured off the UI thread (Task.Run) and applied back where Measure was called: the
// view calls it on the UI thread, so the await resumes there. A folder that cannot be walked says
// "Couldn't ...", never zero. Opening the folder is the view's (File Explorer): the model only
// hands it the path. Paths never reach the log (they carry the user's name).
using System.Globalization;

namespace Inkwell.Core.Screens;

/// <param name="Library">The library database (transcripts, notes, summaries).</param>
/// <param name="Recordings">Meeting recordings (everything else in the data folder).</param>
/// <param name="Models">Speech models.</param>
public readonly record struct StorageSizes(long Library, long Recordings, long Models);

/// <param name="dataDirectory">The core's data folder (InkConfig.DataDir).</param>
/// <param name="modelsDirectory">Models kept elsewhere (InkConfig.ModelsDir), or null for the data folder's models.</param>
/// <param name="reveal">Opens a folder in File Explorer (the view's: explorer.exe); null where nothing can.</param>
public sealed class StorageModel(
    string? dataDirectory, string? modelsDirectory, Action<string>? reveal = null, ScreenLog? log = null) : ObservableModel
{
    public const string FailedText = "Couldn't measure the library's folder.";
    public const string RetentionDetail = "Older meetings and dictations are deleted with their recordings: their words are overwritten in the library's files, not only hidden. Anything you imported is kept. Nothing is deleted while it is forever.";

    private readonly ScreenLog log = log ?? ScreenLog.System;
    private bool measuring;

    public string? DataDirectory { get; } = dataDirectory;

    public string? ModelsDirectory { get; } = modelsDirectory;

    /// <summary>The last measure; null until one finishes (the view shows it measuring) or when it failed.</summary>
    public StorageSizes? Sizes { get; private set; }

    /// <summary>The last measure failed: the sizes are not known, which is not the same as zero.</summary>
    public bool Failed { get; private set; }

    /// <summary>Whether "Show in File Explorer" can open anything.</summary>
    public bool CanReveal => DataDirectory is not null && reveal is not null;

    /// <summary>Measures off the UI thread; the task ends once the sizes are applied. A measure already running is not doubled.</summary>
    public async Task Measure()
    {
        if (measuring)
        {
            return;
        }
        if (DataDirectory is not string data)
        {
            Failed = true;
            Changed();
            return;
        }
        measuring = true;
        var models = ModelsDirectory ?? Path.Combine(data, "models");
        try
        {
            Sizes = await Task.Run(() => MeasureFolders(data, models)).ConfigureAwait(true);
            Failed = false;
        }
        catch (Exception e)
        {
            // The kind only: a message can name the path.
            log.Write($"storage measure failed: {e.GetType().Name}");
            Sizes = null;
            Failed = true;
        }
        finally
        {
            measuring = false;
        }
        Changed();
    }

    /// <summary>
    /// Sums the files under <paramref name="data"/>: the library's database files, the models (under
    /// <paramref name="models"/>, which may be elsewhere), and everything else as recordings, bar the
    /// core's lock and socket. Throws when <paramref name="data"/> cannot be walked; a models folder
    /// that does not exist yet holds nothing.
    /// </summary>
    public static StorageSizes MeasureFolders(string data, string models)
    {
        var modelsRoot = Path.TrimEndingDirectorySeparator(Path.GetFullPath(models)) + Path.DirectorySeparatorChar;
        long library = 0, recordings = 0;
        foreach (var file in Files(data))
        {
            var name = file.Name;
            if (file.FullName.StartsWith(modelsRoot, StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }
            if (name.StartsWith("library.sqlite", StringComparison.OrdinalIgnoreCase))
            {
                library += file.Length;
            }
            else if (!name.Equals("inkwell.lock", StringComparison.OrdinalIgnoreCase) && !name.Equals("inkwell.sock", StringComparison.OrdinalIgnoreCase))
            {
                recordings += file.Length;
            }
        }
        var modelBytes = Directory.Exists(models) ? Files(models).Sum(f => f.Length) : 0;
        return new StorageSizes(library, recordings, modelBytes);
    }

    private static IEnumerable<FileInfo> Files(string root) =>
        // A missing root throws (DirectoryNotFoundException); a subfolder that can't be read is skipped.
        // Links are not followed, so nothing is counted twice or outside the folder.
        new DirectoryInfo(root).EnumerateFiles("*", new EnumerationOptions
        {
            RecurseSubdirectories = true,
            IgnoreInaccessible = true,
            AttributesToSkip = FileAttributes.ReparsePoint,
        });

    /// <summary>Opens the library's folder in File Explorer.</summary>
    public void ShowInFileExplorer()
    {
        if (DataDirectory is string data)
        {
            reveal?.Invoke(data);
        }
    }

    /// <summary>
    /// A size as Windows writes one (1,024 bytes a KB, three significant digits): "0 bytes",
    /// "512 bytes", "1.50 KB", "2.34 GB". Digits in <paramref name="format"/>'s culture (the current one by default).
    /// </summary>
    public static string Size(long bytes, IFormatProvider? format = null)
    {
        format ??= CultureInfo.CurrentCulture;
        if (bytes < 1024)
        {
            return string.Format(format, "{0} bytes", Math.Max(0, bytes));
        }
        string[] units = ["KB", "MB", "GB", "TB", "PB"];
        var value = bytes / 1024.0;
        var unit = 0;
        while (value >= 1000 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }
        // Three significant digits, truncated (as Explorer's details pane): 1.99 KB, never 2.00 KB for 2,047 bytes.
        var decimals = value >= 100 ? 0 : value >= 10 ? 1 : 2;
        var scale = Math.Pow(10, decimals);
        var shown = Math.Floor(value * scale) / scale;
        return string.Format(format, "{0} {1}", shown.ToString($"F{decimals}", format), units[unit]);
    }
}
