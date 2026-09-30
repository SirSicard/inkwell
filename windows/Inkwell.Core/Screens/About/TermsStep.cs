// The first run's terms step (Windows only). The licences of the Windows App SDK (section
// 3(b)(ii)) and of the Windows SDK (Distribution Requirements) ask that the people who use an app
// shipping their code agree to terms that protect it at least as much, and Velopack's installer has
// no page to agree on. So before the app makes, shows or starts anything else, this step shows the
// terms sentence (Notices.WindowsAppSdkTerms) and each Microsoft licence in full, as About shows
// them, with Agree and Quit. Agree starts the app; Quit exits it with nothing started.
//
// The agreement is remembered with the version of the terms agreed to (a hash of the sentence and
// the two licences), so changed terms ask again. It is kept in a one-line file in the library's
// folder, beside the core's store rather than in it: the core's settings are read only by a
// running core, and starting the core starts work of its own (the library's retention sweep, and
// meeting detection wherever the platform has a detector), none of which may run before Agree.

namespace Inkwell.Core.Screens;

/// <summary>Where the agreement is kept: the version agreed to, in a file of its own.</summary>
/// <param name="path">The file (in the library's folder, as <see cref="FileName"/>).</param>
public sealed class TermsRecord(string path)
{
    /// <summary>The file's name in the library's folder.</summary>
    public const string FileName = "terms-agreed.txt";

    /// <summary>The version agreed to, or null when none is recorded. Other failures to read it throw.</summary>
    public string? Read()
    {
        try
        {
            return File.ReadAllText(path).Trim();
        }
        catch (Exception e) when (e is FileNotFoundException or DirectoryNotFoundException)
        {
            return null;
        }
    }

    /// <summary>Records <paramref name="version"/> as agreed to. A failure to write it throws.</summary>
    public void Write(string version)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(path))!);
        File.WriteAllText(path, version + "\n");
    }
}

/// <summary>The terms step's state. UI thread only, like every screen model.</summary>
public sealed class TermsStep
{
    private readonly TermsRecord? record;
    private readonly Action start;
    private readonly Action quit;
    private readonly ScreenLog log;
    private readonly string version;
    private bool answered;

    /// <param name="record">Where the agreement is kept; null when the library's folder is not known (the step then shows at every launch).</param>
    /// <param name="start">Everything else the app does: runs once, on Agree, or at <see cref="Launch"/> when the terms were agreed to.</param>
    /// <param name="quit">Exits the app: runs on Quit, and nothing was started.</param>
    /// <param name="version">The terms' version; <see cref="CurrentVersion"/> but in tests.</param>
    public TermsStep(TermsRecord? record, Action start, Action quit, ScreenLog? log = null, string? version = null)
    {
        ArgumentNullException.ThrowIfNull(start);
        ArgumentNullException.ThrowIfNull(quit);
        this.record = record;
        this.start = start;
        this.quit = quit;
        this.log = log ?? ScreenLog.System;
        this.version = version ?? CurrentVersion;
    }

    /// <summary>Whether the step is up: the terms are not agreed to in this version, and not yet answered.</summary>
    public bool Showing { get; private set; }

    /// <summary>
    /// At launch, before anything else: the app starts when these terms were agreed to; otherwise
    /// the step shows, and nothing starts until Agree. A record that cannot be read shows the step.
    /// </summary>
    public void Launch()
    {
        string? agreed = null;
        if (record is null)
        {
            log.Write("the library's folder is not known: the terms step shows, and an agreement is not recorded");
        }
        else
        {
            try
            {
                agreed = record.Read();
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                log.Write($"couldn't read the terms agreement ({e.GetType().Name}): the terms step shows");
            }
        }
        if (agreed == version)
        {
            Answer(start);
            return;
        }
        Showing = true;
    }

    /// <summary>
    /// Agree: recorded with this version, then the app starts. A record that cannot be written is
    /// logged, and the app still starts (the user did agree); the step shows again next launch.
    /// </summary>
    public void Agree()
    {
        if (!Showing)
        {
            return;
        }
        try
        {
            record?.Write(version);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            log.Write($"couldn't record the terms agreement ({e.GetType().Name}): the terms step shows again next launch");
        }
        Showing = false;
        Answer(start);
    }

    /// <summary>Quit: nothing recorded, nothing started; the app exits.</summary>
    public void Quit()
    {
        if (!Showing)
        {
            return;
        }
        Showing = false;
        Answer(quit);
    }

    /// <summary>Runs the answer's action once: a second Agree or Quit does nothing.</summary>
    private void Answer(Action action)
    {
        if (answered)
        {
            return;
        }
        answered = true;
        action();
    }

    // The step's words and texts.

    public const string Title = "Microsoft's terms";

    /// <summary>The terms sentence, as About shows it (Notices.WindowsAppSdkTerms).</summary>
    public static string Sentence => Notices.WindowsAppSdkTerms;

    /// <summary>The two Microsoft licences the sentence names, each as About's row shows it, text in full.</summary>
    public static IReadOnlyList<NoticeRow> Licences { get; } =
        [.. new[] { "windows-app-sdk", "windows-sdk-net" }
            .Select(id => Notices.Components.Single(c => c.Id == id))
            .Select(c => new NoticeRow(c.Title, c.Role, c.Text))];

    public const string AgreeTitle = "Agree";

    public const string QuitTitle = "Quit";

    /// <summary>
    /// The version of the terms: a hash of the sentence and the two licences' texts, so that any
    /// change to what the user agrees to asks again.
    /// </summary>
    public static string CurrentVersion { get; } = VersionOf(Sentence, Notices.WindowsAppSdkLicence, Notices.WindowsSdkNetText);

    internal static string VersionOf(params string[] texts) =>
        "sha256:" + Convert.ToHexStringLower(System.Security.Cryptography.SHA256.HashData(
            System.Text.Encoding.UTF8.GetBytes(string.Join('\0', texts))));
}
