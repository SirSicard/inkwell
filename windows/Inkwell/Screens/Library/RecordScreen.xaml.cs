// The record's code: it shows LibraryModel.Document and sends the user's presses to the model.
// The record is built once per answer (a new document), not per change of the model; the ledger
// marks the line under the playhead when the player bar's timer ticks (only while playing). Space
// plays and pauses while no text field has the focus (a focused button keeps its own Space).
using Inkwell.Core.Screens;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;

namespace Inkwell.Screens;

public sealed partial class RecordScreen : UserControl
{
    private readonly LibraryModel _library;
    private readonly Func<string?> _summaryOffNote;
    private readonly WindowPresence _presence;
    private RecordDocument? _shown;
    private RecordPlayer? _barPlayer;
    private PlayerBar? _bar;

    /// <param name="library">The Library's model: its open record, player and commands.</param>
    /// <param name="summaryOffNote">The AI settings' "Summaries are off…" note, or null (AiSettings.SummaryOffNote).</param>
    /// <param name="presence">Whether the window is on screen (the player's playhead draws only then).</param>
    public RecordScreen(LibraryModel library, Func<string?> summaryOffNote, WindowPresence presence)
    {
        ArgumentNullException.ThrowIfNull(presence);
        _presence = presence;
        ArgumentNullException.ThrowIfNull(library);
        ArgumentNullException.ThrowIfNull(summaryOffNote);
        _library = library;
        _summaryOffNote = summaryOffNote;
        InitializeComponent();
        FailedTitle.Text = LibraryModel.OpenFailedTitle;
        TryAgain.Content = LibraryModel.TryAgainText;
        NoSummaryTitle.Text = RecordDocument.NoSummaryTitle;
        EmptyLedger.Text = RecordDocument.EmptyLedgerText;
        LedgerEyebrow.Content = Parts.Eyebrow("What was said");
        _library.PropertyChanged += (_, _) => Render();
        KeyDown += OnKeyDown;
        ActualThemeChanged += (_, _) =>
        {
            _shown = null;
            Render();
        };
        Render();
    }

    private void Render()
    {
        var document = _library.Document;
        var failure = document is null ? _library.OpenFailure : null;
        var opening = document is null && failure is null && _library.Selected is not null;
        Shown.Visibility = document is null ? Visibility.Collapsed : Visibility.Visible;
        Failed.Visibility = failure is null ? Visibility.Collapsed : Visibility.Visible;
        FailedText.Text = failure ?? "";
        Opening.IsActive = opening;
        Opening.Visibility = opening ? Visibility.Visible : Visibility.Collapsed;
        if (document is null)
        {
            _shown = null;
            ShowPlayer(null, null);
            return;
        }
        if (!ReferenceEquals(document, _shown))
        {
            _shown = document;
            Fill(document);
        }
        ShowPlayer(_library.Player, document);
    }

    private void Fill(RecordDocument document)
    {
        RecordTitle.Text = LibraryFormat.Title(document.Record);
        RecordLine.Text = document.HeaderLine(_library.Now(), _library.Calendar);
        var hasSummary = document.Summary is not null;
        CopySummaryButton.IsEnabled = hasSummary;
        CopySummaryItem.IsEnabled = hasSummary;
        NotesTab.Text = document.TabTitle(RecordTab.Notes);
        SummaryTab.Text = document.TabTitle(RecordTab.Summary);
        OwedTab.Text = document.TabTitle(RecordTab.Owed);
        FillNotes(document);
        FillLedger(document);
        FillSummary(document);
        FillOwed(document);
    }

    private void Play(long ms) => _library.PlayFrom(ms);

    /// <summary>Space plays or pauses the record, unless a text field has the focus.</summary>
    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Handled || e.Key != Windows.System.VirtualKey.Space || _library.Player is not { } player)
        {
            return;
        }
        if (FocusManager.GetFocusedElement(XamlRoot) is TextBox or RichEditBox or PasswordBox or AutoSuggestBox)
        {
            return;
        }
        player.Toggle();
        e.Handled = true;
    }

    // Notes and transcript

    private void FillNotes(RecordDocument document)
    {
        NotesStack.Children.Clear();
        NotesStack.Children.Add(Parts.Eyebrow(document.NotesHeading));
        if (document.NoNotesText is string none)
        {
            NotesStack.Children.Add(Parts.Text(none, "InkCaptionStyle"));
        }
        foreach (var entry in document.Merged)
        {
            NotesStack.Children.Add(entry.Kind switch
            {
                MergedKind.Note => Note(entry),
                MergedKind.Said => Said(entry),
                _ => Owed(entry),
            });
        }
    }

    private Grid Note(MergedEntry entry)
    {
        var words = Parts.Text(entry.Text, "InkReadingStyle");
        words.FontSize = 16;
        var row = Parts.WithChip(words, entry.AtMs, Play, this);
        row.Margin = new Thickness(0, 4, 0, 0);
        return row;
    }

    private Grid Said(MergedEntry entry)
    {
        var speaker = entry.Speaker ?? Speaker.You;
        var words = Parts.Text("", "InkReadingStyle");
        words.FontSize = 15;
        words.Foreground = Parts.Brush("InkSecondaryTextBrush", this);
        words.Inlines.Add(new Run
        {
            Text = $"{speaker.Label}: ",
            FontWeight = FontWeights.SemiBold,
            Foreground = Parts.Brush("InkTextBrush", this),
        });
        words.Inlines.Add(new Run { Text = entry.Text });
        return Parts.WithChip(words, entry.AtMs, Play, this);
    }

    private Grid Owed(MergedEntry entry)
    {
        var row = new Grid { ColumnSpacing = 6 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var mark = new FontIcon { Glyph = "", FontSize = 13, Foreground = Parts.Brush("InkTextBrush", this), VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 4, 0, 0) };
        AutomationProperties.SetName(mark, "Owed");
        var words = Parts.Text(entry.Text, "InkReadingStyle");
        words.FontSize = 15;
        var line = Parts.WithChip(words, entry.AtMs, Play, this);
        Grid.SetColumn(line, 1);
        row.Children.Add(mark);
        row.Children.Add(line);
        return row;
    }

    private void FillLedger(RecordDocument document)
    {
        LedgerStatus.Text = document.LedgerStatus(_library.Calendar);
        EmptyLedger.Visibility = document.Ledger.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        LedgerList.ItemsSource = document.Ledger.Select(l => new LedgerItem(l)).ToList();
        MarkLine();
    }

    /// <summary>The line under the playhead, once the playhead has been put somewhere.</summary>
    private void MarkLine()
    {
        var line = _library.PlayheadLine() ?? -1;
        if (LedgerList.SelectedIndex != line)
        {
            LedgerList.SelectedIndex = line;
        }
    }

    private void OnLineClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is LedgerItem item)
        {
            _library.PlayFrom(item.Line.StartMs);
        }
    }

    // Summary

    private void FillSummary(RecordDocument document)
    {
        SummaryText.Blocks.Clear();
        if (document.Summary is { } summary)
        {
            // Model text: words and styles only; no run carries a link (SummaryDocument).
            if (summary.Headline is { } headline)
            {
                SummaryText.Blocks.Add(Paragraph(headline, 20, FontWeights.Medium, new Thickness(0, 0, 0, 10)));
            }
            foreach (var block in summary.Blocks)
            {
                SummaryText.Blocks.Add(block switch
                {
                    SummaryBlock.Heading heading => Paragraph(heading.Text, heading.Level <= 2 ? 20 : 17, FontWeights.SemiBold, new Thickness(0, 14, 0, 4)),
                    SummaryBlock.Item item => Paragraph(item.Text, 16, FontWeights.Normal, new Thickness(22, 2, 0, 2), item.Marker),
                    _ => Paragraph(block.Text, 16, FontWeights.Normal, new Thickness(0, 0, 0, 10)),
                });
            }
        }
        SummaryText.Visibility = document.Summary is null ? Visibility.Collapsed : Visibility.Visible;
        NoSummary.Visibility = document.Summary is null ? Visibility.Visible : Visibility.Collapsed;
        NoSummaryText.Text = RecordDocument.NoSummaryText(_summaryOffNote());

        CitedStack.Children.Clear();
        if (document.Decisions.Count > 0)
        {
            CitedStack.Children.Add(Spaced(Parts.Eyebrow("Decided, and where")));
            foreach (var item in document.Decisions)
            {
                CitedStack.Children.Add(Cited(item.Text, item.CitedLine));
            }
        }
        if (document.CitedOwed.Count > 0)
        {
            CitedStack.Children.Add(Spaced(Parts.Eyebrow("Where it was said")));
            foreach (var item in document.CitedOwed)
            {
                CitedStack.Children.Add(Cited(item.Text, item.CitedLine));
            }
        }
    }

    private static TextBlock Spaced(TextBlock eyebrow)
    {
        eyebrow.Margin = new Thickness(0, 14, 0, 0);
        return eyebrow;
    }

    private Paragraph Paragraph(SummaryText text, double size, Windows.UI.Text.FontWeight weight, Thickness margin, string? marker = null)
    {
        var paragraph = new Paragraph { FontSize = size, FontWeight = weight, Margin = margin };
        if (marker is not null)
        {
            paragraph.TextIndent = -22;
            paragraph.Inlines.Add(new Run { Text = marker + " ", Foreground = Parts.Brush("InkSecondaryTextBrush", this) });
        }
        foreach (var run in text.Runs)
        {
            var inline = new Run { Text = run.Text };
            if (run.Bold)
            {
                inline.FontWeight = FontWeights.SemiBold;
            }
            if (run.Italic)
            {
                inline.FontStyle = Windows.UI.Text.FontStyle.Italic;
            }
            if (run.Code)
            {
                inline.FontFamily = Parts.Font("InkMonoFontFamily");
            }
            if (run.Strikethrough)
            {
                inline.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
            }
            paragraph.Inlines.Add(inline);
        }
        return paragraph;
    }

    /// <summary>A promise from the summary with the line it came from, so a mismatch shows at a glance.</summary>
    private StackPanel Cited(string text, LedgerLine? line)
    {
        var item = new StackPanel { Spacing = 4, Padding = new Thickness(0, 4, 0, 4) };
        var words = Parts.Text(text);
        words.FontWeight = FontWeights.Medium;
        item.Children.Add(words);
        if (line is not null)
        {
            var quote = Parts.Text(RecordDocument.QuoteOf(line), "InkReadingStyle");
            quote.FontSize = 15;
            quote.Foreground = Parts.Brush("InkSecondaryTextBrush", this);
            item.Children.Add(Parts.WithChip(quote, line.StartMs, Play, this));
        }
        return item;
    }

    // Owed

    private void FillOwed(RecordDocument document)
    {
        OwedStack.Children.Clear();
        if (document.Owed.Count == 0)
        {
            OwedStack.Children.Add(Parts.Text(RecordDocument.NothingOwedText, "InkCaptionStyle"));
        }
        foreach (var item in document.Owed)
        {
            var row = new Grid { ColumnSpacing = 12 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var toggle = new Button
            {
                Content = new FontIcon { Glyph = item.Done ? "" : "", FontSize = 18 },
                Padding = new Thickness(4),
                MinWidth = 0,
                MinHeight = 0,
                Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
                BorderThickness = new Thickness(0),
                VerticalAlignment = VerticalAlignment.Top,
            };
            AutomationProperties.SetName(toggle, item.ToggleLabel);
            var id = item.Id;
            var done = item.Done;
            toggle.Click += (_, _) => _library.SetDone(id, !done);
            var words = new StackPanel { Spacing = 3 };
            var text = Parts.Text(item.Text);
            if (item.Done)
            {
                text.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
                text.Foreground = Parts.Brush("InkSecondaryTextBrush", this);
            }
            words.Children.Add(text);
            var meta = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            if (item.MetaLine.Length > 0)
            {
                meta.Children.Add(Parts.Text(item.MetaLine, "InkTimestampStyle"));
            }
            if (item.AtMs is long at)
            {
                meta.Children.Add(Parts.Chip(at, Play, this));
            }
            words.Children.Add(meta);
            Grid.SetColumn(words, 1);
            row.Children.Add(toggle);
            row.Children.Add(words);
            OwedStack.Children.Add(row);
        }
    }

    // The player

    private void ShowPlayer(RecordPlayer? player, RecordDocument? document)
    {
        if (ReferenceEquals(player, _barPlayer))
        {
            if (document is not null)
            {
                _bar?.Update(document);
            }
            return;
        }
        _barPlayer = player;
        _bar = player is null || document is null ? null : new PlayerBar(player, document, _presence);
        if (_bar is not null)
        {
            _bar.Ticked += (_, _) => MarkLine();
        }
        PlayerHost.Content = _bar;
    }

    // Presses

    private void OnTabChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        if (NotesPanel is null || SummaryPanel is null || OwedPanel is null)
        {
            // The first tab is selected while the page is still being built.
            return;
        }
        var selected = sender.SelectedItem;
        NotesPanel.Visibility = selected == NotesTab ? Visibility.Visible : Visibility.Collapsed;
        SummaryPanel.Visibility = selected == SummaryTab ? Visibility.Visible : Visibility.Collapsed;
        OwedPanel.Visibility = selected == OwedTab ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnCopySummary(object sender, RoutedEventArgs e)
    {
        if (_library.Document?.Summary is { } summary)
        {
            ShowCopy(TextClipboard.Copy(summary.PlainText));
        }
    }

    private void OnCopyTranscript(object sender, RoutedEventArgs e)
    {
        if (_library.Document is { } document)
        {
            ShowCopy(TextClipboard.Copy(document.TranscriptText));
        }
    }

    private void OnTryAgain(object sender, RoutedEventArgs e) => _library.Reopen();

    /// <summary>A refused copy says so under the title; the next copy that works clears it.</summary>
    private void ShowCopy(bool copied)
    {
        CopyFailure.Text = copied ? "" : TextClipboard.Failed;
        CopyFailure.Visibility = copied ? Visibility.Collapsed : Visibility.Visible;
    }
}
