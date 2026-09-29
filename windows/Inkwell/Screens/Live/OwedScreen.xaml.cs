// The Owed screen's view: lists again when shown (owed.Load on Loaded), and renders the OwedModel on
// its change signal. The groups, rows and words are the model's; the summary and due labels are
// computed for now when the model changes (nothing ticks: a label that crosses midnight updates
// with the next change or the next visit).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class OwedScreen : UserControl
{
    private readonly OwedModel owed;
    private readonly Action<string, long> openRecordAt;
    /// <summary>The rows shown, by id: what a row's buttons act on.</summary>
    private Dictionary<string, OwedRow> rows = [];

    /// <param name="openRecordAt">Opens a record at a time into it, ms (the Library's record screen).</param>
    public OwedScreen(OwedModel owed, Action<string, long> openRecordAt)
    {
        ArgumentNullException.ThrowIfNull(owed);
        ArgumentNullException.ThrowIfNull(openRecordAt);
        this.owed = owed;
        this.openRecordAt = openRecordAt;
        InitializeComponent();
        owed.PropertyChanged += (_, _) => Render();
        Loaded += (_, _) =>
        {
            owed.Load();
            Render();
        };
        Render();
    }

    private static void Show(TextBlock block, string? text)
    {
        block.Text = text ?? "";
        block.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
    }

    private void Render()
    {
        var now = DateTimeOffset.Now;
        Show(SummaryLine, owed.Loaded ? owed.Summary(now) : null);
        Show(FailureLine, owed.Failure);
        Show(LoadFailureLine, owed.LoadFailure);
        SuggestionRows.ItemsSource = owed.Suggestions.ToList();
        LoadingRing.IsActive = owed.Loading;
        LoadingRing.Visibility = owed.Loading ? Visibility.Visible : Visibility.Collapsed;
        EmptyLine.Visibility = owed.Loaded && owed.Items.IsEmpty ? Visibility.Visible : Visibility.Collapsed;
        var groups = owed.Groups(now);
        rows = groups.SelectMany(g => g.Rows).ToDictionary(r => r.Id);
        GroupRows.ItemsSource = groups;
    }

    private static string? Tagged(object sender) => (sender as FrameworkElement)?.Tag as string;

    private void OnRowDone(object sender, RoutedEventArgs e)
    {
        if (Tagged(sender) is string id)
        {
            owed.MarkDone(id);
        }
    }

    private void OnSaidAt(object sender, RoutedEventArgs e)
    {
        if (Tagged(sender) is string id && rows.TryGetValue(id, out var row) && row.SaidAtMs is long at)
        {
            openRecordAt(row.Record, at);
        }
    }

    private void OnSuggestionDone(object sender, RoutedEventArgs e)
    {
        if (Tagged(sender) is string id && owed.Suggestions.Find(s => s.Id == id) is LooksDone suggestion)
        {
            owed.MarkDone(suggestion.Commitment);
        }
    }

    private void OnNotYet(object sender, RoutedEventArgs e)
    {
        if (Tagged(sender) is string id && owed.Suggestions.Find(s => s.Id == id) is LooksDone suggestion)
        {
            owed.NotYet(suggestion);
        }
    }
}

/// <summary>The Owed screen's x:Bind functions.</summary>
public static class OwedViews
{
    public static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility Collapsed(bool hidden) => hidden ? Visibility.Collapsed : Visibility.Visible;

    public static Visibility Shown(string? text) => string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;

    /// <summary>The chip tones, as the Mac's: overdue in the alert colour, no date neutral, the rest "due".</summary>
    public static Visibility AlertChip(DueLabel due) => Visible(due is { IsOverdue: true });

    public static Visibility NeutralChip(DueLabel due) => Visible(due is DueLabel.Undated);

    public static Visibility DueChip(DueLabel due) => Visible(due is not null && !due.IsOverdue && due is not DueLabel.Undated);

    /// <summary>The round button's name, which Narrator reads.</summary>
    public static string MarkDoneName(string text) => $"Mark done: {text}";
}
