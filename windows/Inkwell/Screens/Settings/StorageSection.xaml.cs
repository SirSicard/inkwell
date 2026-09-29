// Settings > Storage. Retention is the meeting model's setting (retention.days), as on the Mac;
// the folder and its sizes are the storage model's, measured each time the section appears. The
// ring spins only while a measure runs.
using System.Globalization;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class StorageSection : UserControl
{
    private readonly StorageModel storage;
    private readonly MeetingModel meetings;
    private bool measuring;
    private bool rendering;

    public StorageSection(StorageModel storage, MeetingModel meetings)
    {
        this.storage = storage ?? throw new ArgumentNullException(nameof(storage));
        this.meetings = meetings ?? throw new ArgumentNullException(nameof(meetings));
        InitializeComponent();
        Keep.ItemsSource = Retentions.All.Select(r => r.Title()).ToList();
        storage.PropertyChanged += (_, _) => Render();
        meetings.PropertyChanged += (_, _) => Render();
        Render();
    }

    public static string RetentionDetail => StorageModel.RetentionDetail;

    public static string FailedText => StorageModel.FailedText;

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        measuring = true;
        Render();
        await storage.Measure().ConfigureAwait(true);
        measuring = false;
        Render();
    }

    private void Render()
    {
        rendering = true;
        Keep.IsEnabled = meetings.Retention is not null;
        Keep.SelectedIndex = meetings.Retention is Retention kept ? Retentions.All.ToList().IndexOf(kept) : 0;
        rendering = false;

        Folder.Visibility = storage.DataDirectory is null ? Visibility.Collapsed : Visibility.Visible;
        FolderPath.Text = storage.DataDirectory ?? "";
        Reveal.IsEnabled = storage.CanReveal;

        var sizes = storage.Sizes;
        Failed.Visibility = storage.Failed ? Visibility.Visible : Visibility.Collapsed;
        var waiting = sizes is null && !storage.Failed;
        Measuring.IsActive = waiting && measuring;
        Measuring.Visibility = waiting ? Visibility.Visible : Visibility.Collapsed;
        Sizes.Visibility = sizes is null ? Visibility.Collapsed : Visibility.Visible;
        if (sizes is StorageSizes s)
        {
            Show(LibrarySize, "Library", s.Library);
            Show(RecordingsSize, "Recordings", s.Recordings);
            Show(ModelsSize, "Models", s.Models);
        }
    }

    private static void Show(TextBlock size, string label, long bytes)
    {
        size.Text = StorageModel.Size(bytes, CultureInfo.CurrentCulture);
        AutomationProperties.SetName(size, $"{label}: {size.Text}");
    }

    private void OnKeepChanged(object sender, SelectionChangedEventArgs e)
    {
        if (rendering || Keep.SelectedIndex < 0)
        {
            return;
        }
        var chosen = Retentions.All[Keep.SelectedIndex];
        if (chosen != meetings.Retention)
        {
            meetings.SetRetention(chosen);
        }
    }

    private void OnReveal(object sender, RoutedEventArgs e) => storage.ShowInFileExplorer();
}
