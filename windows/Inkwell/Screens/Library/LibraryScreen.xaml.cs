// The Library's code: it shows LibraryModel and sends it the user's presses. The list is built
// again only when the model's records or matches change (a new answer), so typing in the search
// box or moving the playhead never rebuilds it.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Inkwell.Screens;

public sealed partial class LibraryScreen : UserControl
{
    private readonly LibraryModel _library;
    private readonly RecordScreen _record;
    private readonly Dictionary<RecordKind, ToggleButton> _chips = [];
    private IReadOnlyList<RecordRow>? _listed;
    private IReadOnlyList<SearchHit>? _matched;
    private bool _syncing;

    /// <param name="library">The Library's model (shared with Today, which opens records in it).</param>
    /// <param name="summaryOffNote">The AI settings' "Summaries are off…" note, or null (AiSettings.SummaryOffNote).</param>
    public LibraryScreen(LibraryModel library, Func<string?> summaryOffNote)
    {
        ArgumentNullException.ThrowIfNull(library);
        _library = library;
        InitializeComponent();
        _record = new RecordScreen(library, summaryOffNote);
        SearchBox.PlaceholderText = LibraryModel.SearchPrompt;
        AutomationProperties.SetName(SearchBox, LibraryModel.SearchPrompt);
        ListFailedText.Text = LibraryModel.ListFailedText;
        ListRetry.Content = LibraryModel.TryAgainText;
        NothingTitle.Text = LibraryModel.NothingSelectedTitle;
        NothingDetail.Text = LibraryModel.NothingSelectedDetail;
        foreach (var kind in LibraryModel.Kinds)
        {
            var chip = new ToggleButton { Content = kind.Title, CornerRadius = new CornerRadius(14), MinHeight = 28, Padding = new Thickness(10, 3, 10, 4) };
            chip.Click += (_, _) => _library.ToggleFilter(kind.Kind);
            _chips[kind.Kind] = chip;
            Chips.Children.Add(chip);
        }
        _library.PropertyChanged += (_, _) => Render();
        Loaded += (_, _) => _library.RefreshList();
        Render();
    }

    private void Render()
    {
        ColumnTitle.Text = _library.ColumnTitle;
        Count.Text = _library.CountText;
        AutomationProperties.SetName(Count, _library.CountLabel);
        if (SearchBox.Text != _library.Query)
        {
            // The title bar's search box asked: show its words here too.
            SearchBox.Text = _library.Query;
        }
        foreach (var kind in LibraryModel.Kinds)
        {
            var chip = _chips[kind.Kind];
            chip.IsChecked = _library.Filter == kind.Kind;
            AutomationProperties.SetHelpText(chip, _library.ChipHint(kind));
        }

        var searching = _library.IsSearching;
        Chips.Visibility = searching ? Visibility.Collapsed : Visibility.Visible;
        var failed = !searching && _library.ListLoad == LibraryLoad.Failed;
        var empty = !searching && !failed && _library.ShowsEmpty;
        ListFailed.Visibility = failed ? Visibility.Visible : Visibility.Collapsed;
        Empty.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        RecordList.Visibility = !searching && !failed && !empty ? Visibility.Visible : Visibility.Collapsed;
        (EmptyTitle.Text, EmptyDetail.Text) = _library.EmptyText;
        ShowRecords();

        var hits = searching && _library.Hits.Count > 0;
        SearchStatus.Visibility = searching && !hits ? Visibility.Visible : Visibility.Collapsed;
        SearchStatus.Text = _library.SearchStatus;
        HitList.Visibility = hits ? Visibility.Visible : Visibility.Collapsed;
        if (!ReferenceEquals(_matched, _library.Hits))
        {
            _matched = _library.Hits;
            var now = _library.Now();
            HitList.ItemsSource = _library.Hits.Select(h => new HitItem(h, LibraryFormat.HitLine(h, now, _library.Calendar))).ToList();
        }

        var selected = _library.Selected is not null;
        NothingSelected.Visibility = selected ? Visibility.Collapsed : Visibility.Visible;
        RecordHost.Content = selected ? _record : null;
    }

    private void ShowRecords()
    {
        _syncing = true;
        if (!ReferenceEquals(_listed, _library.Records))
        {
            _listed = _library.Records;
            var now = _library.Now();
            RecordList.ItemsSource = _library.Records
                .Select(r => new RecordListItem(r, LibraryFormat.ListLine(r, now, _library.Calendar)))
                .ToList();
        }
        if (RecordList.ItemsSource is List<RecordListItem> items)
        {
            RecordList.SelectedItem = items.Find(i => i.Id == _library.Selected);
        }
        MoreButton.Visibility = _library.HasMore ? Visibility.Visible : Visibility.Collapsed;
        MoreButton.Content = _library.MoreText;
        MoreButton.IsEnabled = _library.MoreLoad != LibraryLoad.Loading;
        _syncing = false;
    }

    private void OnSearchChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason == AutoSuggestionBoxTextChangeReason.UserInput)
        {
            _library.Query = sender.Text;
        }
    }

    private void OnRecordSelected(object sender, SelectionChangedEventArgs e)
    {
        // Arrow keys and clicks move the selection; the model opens what is selected.
        if (!_syncing && RecordList.SelectedItem is RecordListItem item)
        {
            _library.Open(item.Id);
        }
    }

    private void OnHitClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is HitItem hit)
        {
            _library.Open(hit.Record, hit.StartMs);
        }
    }

    private void OnMore(object sender, RoutedEventArgs e) => _library.LoadMore();

    private void OnRetryList(object sender, RoutedEventArgs e) => _library.RefreshList();
}
