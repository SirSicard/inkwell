// What the Settings sections of this part (Models, Modes, Snippets, Voice commands, Meetings,
// Storage) share: x:Bind functions, an app's icon tile, File Explorer, keeping keyboard focus on a
// list row across a re-render, and the rows' view items (the model's rows plus what a template
// can't compute from one property).
using System.Diagnostics;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage;
using Windows.Storage.FileProperties;

namespace Inkwell.Screens;

/// <summary>x:Bind functions for the sections' templates.</summary>
public static class SettingsFormat
{
    public static Visibility Shows(string? text) => string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;

    public static Visibility ShowsIf(bool on) => on ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility HidesIf(bool on) => on ? Visibility.Collapsed : Visibility.Visible;

    public static bool Not(bool on) => !on;

    /// <summary>Off rows read in the secondary colour, as on the Mac.</summary>
    public static double Dimmed(bool enabled) => enabled ? 1 : 0.62;

    /// <summary>A snippet read aloud.</summary>
    public static string SnippetName(string trigger, string expansion, string category, bool enabled) =>
        $"Snippet {trigger}{(enabled ? "" : ", off")}: {expansion}{(category.Length == 0 ? "" : $", {category}")}";
}

/// <summary>
/// The id of the row a control sits in: the Tag ({x:Bind Id}) on its row's root. An ItemsRepeater
/// gives an x:Bind template's elements no DataContext (null on a desktop run), so a row's buttons
/// read their row this way; a lookup through DataContext found nothing and the click did nothing.
/// </summary>
public static class RowTag
{
    public static string? Of(object sender)
    {
        for (var element = sender as DependencyObject; element is not null; element = VisualTreeHelper.GetParent(element))
        {
            if (element is FrameworkElement { Tag: string id })
            {
                return id;
            }
        }
        return null;
    }
}

/// <summary>Opens a folder in File Explorer: StorageModel's reveal.</summary>
public static class FileExplorer
{
    public static void Reveal(string folder)
    {
        var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false };
        start.ArgumentList.Add(folder);
        try
        {
            using var _ = Process.Start(start);
        }
        catch (System.ComponentModel.Win32Exception e)
        {
            // Explorer could not start: logged by its error code (never the path). The folder's
            // path stays on screen, selectable.
            ScreenLog.System.Write($"File Explorer could not open the library's folder (error {e.NativeErrorCode})");
        }
    }
}

/// <summary>
/// An app in a mode's row, 24 px: its icon from the file the directory named (the shell's
/// thumbnail of the exe), over a tile with its initial, which shows when there is no icon or it
/// can't be read (as the design's tiles). Named for Narrator and the tooltip.
/// </summary>
public sealed partial class AppIconTile : Grid
{
    public static readonly DependencyProperty AppProperty = DependencyProperty.Register(
        nameof(App), typeof(AppLabel), typeof(AppIconTile), new PropertyMetadata(null, (d, _) => ((AppIconTile)d).Show()));

    /// <summary>Icons already read, by path (null: none could be).</summary>
    private static readonly Dictionary<string, ImageSource?> Icons = new(StringComparer.OrdinalIgnoreCase);

    private readonly TextBlock initial = new()
    {
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
        FontSize = 11,
        FontWeight = FontWeights.SemiBold,
    };
    private readonly Image icon = new() { Width = 24, Height = 24 };

    public AppIconTile()
    {
        Width = 24;
        Height = 24;
        CornerRadius = new CornerRadius(6);
        var tile = new Border { CornerRadius = new CornerRadius(6), Child = initial };
        tile.Background = ThemeBrush("InkChipBrush");
        initial.Foreground = ThemeBrush("InkTextBrush");
        ActualThemeChanged += (_, _) =>
        {
            tile.Background = ThemeBrush("InkChipBrush");
            initial.Foreground = ThemeBrush("InkTextBrush");
        };
        Children.Add(tile);
        Children.Add(icon);
    }

    public AppLabel? App
    {
        get => (AppLabel?)GetValue(AppProperty);
        set => SetValue(AppProperty, value);
    }

    private Brush ThemeBrush(string key)
    {
        var theme = ActualTheme == ElementTheme.Dark ? "Dark" : "Light";
        return (Brush)((ResourceDictionary)Application.Current.Resources.ThemeDictionaries[theme])[key];
    }

    private void Show()
    {
        var app = App;
        icon.Source = null;
        initial.Text = app is null || app.Name.Length == 0 ? "" : app.Name[..1].ToUpperInvariant();
        AutomationProperties.SetName(this, app?.Name ?? "");
        ToolTipService.SetToolTip(this, app?.Name);
        if (app?.IconPath is string path)
        {
            _ = LoadIcon(app, path);
        }
    }

    private async Task LoadIcon(AppLabel app, string path)
    {
        if (!Icons.TryGetValue(path, out var source))
        {
            source = await Thumbnail(path).ConfigureAwait(true);
            Icons[path] = source;
        }
        // Recycled for another app while it loaded: leave it.
        if (ReferenceEquals(App, app))
        {
            icon.Source = source;
        }
    }

    private static async Task<ImageSource?> Thumbnail(string path)
    {
        try
        {
            var file = await StorageFile.GetFileFromPathAsync(path);
            using var thumbnail = await file.GetThumbnailAsync(ThumbnailMode.SingleItem, 32, ThumbnailOptions.UseCurrentScale);
            if (thumbnail is null)
            {
                return null;
            }
            var image = new BitmapImage();
            await image.SetSourceAsync(thumbnail);
            return image;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or System.Runtime.InteropServices.COMException)
        {
            // No icon: the tile's initial shows.
            return null;
        }
    }
}

/// <summary>
/// Keeps keyboard focus on the same control of the same row when a list's rows are replaced (a
/// model sends a whole new list after every change). A row's root carries its id in Tag; the
/// control keeps its x:Name.
/// </summary>
public static class RowFocus
{
    public static (string Id, string Control)? Capture(ItemsRepeater list, XamlRoot? root)
    {
        ArgumentNullException.ThrowIfNull(list);
        if (root is null || Microsoft.UI.Xaml.Input.FocusManager.GetFocusedElement(root) is not FrameworkElement focused || focused.Name.Length == 0)
        {
            return null;
        }
        for (DependencyObject? at = VisualTreeHelper.GetParent(focused); at is not null && !ReferenceEquals(at, list); at = VisualTreeHelper.GetParent(at))
        {
            if (at is FrameworkElement { Tag: string id } row && ReferenceEquals(VisualTreeHelper.GetParent(row), list))
            {
                return (id, focused.Name);
            }
        }
        return null;
    }

    public static void Restore(ItemsRepeater list, (string Id, string Control)? focus, Func<object, string> idOf)
    {
        ArgumentNullException.ThrowIfNull(list);
        ArgumentNullException.ThrowIfNull(idOf);
        if (focus is not var (id, control) || list.ItemsSourceView is not { } items)
        {
            return;
        }
        for (var i = 0; i < items.Count; i++)
        {
            if (idOf(items.GetAt(i)) == id)
            {
                if (list.GetOrCreateElement(i) is FrameworkElement row)
                {
                    row.UpdateLayout();
                    if (row.FindName(control) is Control target)
                    {
                        target.Focus(FocusState.Keyboard);
                    }
                }
                return;
            }
        }
    }
}

/// <summary>A voice command's row as the template shows it.</summary>
public sealed record VoiceCommandItem(VoiceCommandDraft Row)
{
    public string Id => Row.Id;
    public string Triggers => string.Join(" · ", Row.Triggers);
    public string PrimaryTrigger => Row.Triggers.Count > 0 ? Row.Triggers[0] : "";
    public string Aliases => Row.Triggers.Count > 1 ? $"Also: {string.Join(" · ", Row.Triggers.Skip(1))}" : "";
    public string Does => VoiceCommandsModel.Describe(Row);
    public bool Enabled => Row.Enabled;
    public bool NotCarriedOut => !Row.CarriedOut;
    public string Name => VoiceCommandsModel.AccessibilityLabel(Row);
}

/// <summary>A model's row where it can be downloaded, with a Download of its own (ModelRowsView).</summary>
public sealed record ModelRowItem(ModelRow Row)
{
    public string Id => Row.Id;
    public string Text => Row.Text(System.Globalization.CultureInfo.CurrentCulture);
    /// <summary>Where it would come from, beside its Download.</summary>
    public string? From => Row.From;
    public bool ShowsDownload => Row.CanDownload;
    public string DownloadName => Row.DownloadName(System.Globalization.CultureInfo.CurrentCulture);
    public bool Downloading => Row.Progress is not null;
    public double Progress => Row.Progress ?? 0;
    public string ProgressName => Row.ProgressName;
    /// <summary>What the download is doing (a failure is Failure's).</summary>
    public string? Status => Row.CanRetry ? null : Row.Status(System.Globalization.CultureInfo.CurrentCulture);
    public string? Failure => Row.CanRetry ? Row.Status(System.Globalization.CultureInfo.CurrentCulture) : null;
    public bool CanRetry => Row.CanRetry;
    public string RetryName => Row.RetryName;
    public bool ShowsCancel => Row.CanCancel;
    public string CancelName => Row.CancelName;
    public bool ShowsRemove => Row.CanRemove;
    public string RemoveName => Row.RemoveName;
    /// <summary>Why its last Cancel or Remove did nothing.</summary>
    public string? Note => Row.Note;
}

/// <summary>One job's line in Settings > Models.</summary>
/// <param name="HasEngine">Something fills the job (else the line says "Nothing installed yet" or "Checking…", in the secondary colour).</param>
public sealed record CatalogueLineItem(string Title, string Engine, bool HasEngine, string? Accuracy)
{
    public static CatalogueLineItem Of(CatalogueModel model, Job job)
    {
        ArgumentNullException.ThrowIfNull(model);
        var line = model.Line(job);
        return new CatalogueLineItem(CatalogueModel.Title(job), line.EngineText, line.Engine is not null, line.Accuracy);
    }
}
