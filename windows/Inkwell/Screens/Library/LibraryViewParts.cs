// What the Library, Record and player views share: the rows their lists template (each reads as
// its words to Narrator: ListView speaks an item's ToString), the timestamp chip, the eyebrow, the
// clipboard, and the search's one-shot wait on the UI thread. No logic: the words come from the
// models (Inkwell.Core.Screens).
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;

namespace Inkwell.Screens;

/// <summary>A record in the Library's list.</summary>
public sealed class RecordListItem(RecordRow row, string line)
{
    public string Id => row.Record;

    public string Title => LibraryFormat.Title(row);

    public string Line { get; } = line;

    public override string ToString() => $"{Title}, {Line}";
}

/// <summary>A search match.</summary>
public sealed class HitItem(SearchHit hit, string meta)
{
    public string Record => hit.Record;

    public long StartMs => hit.StartMs;

    public string Title => LibraryModel.HitTitle(hit);

    public string Snippet => hit.Snippet;

    public string Meta { get; } = meta;

    public override string ToString() => $"{Title}: {Snippet}, {Meta}";
}

/// <summary>A line of the ledger.</summary>
public sealed class LedgerItem(LedgerLine line)
{
    public LedgerLine Line { get; } = line;

    public string Stamp => LibraryFormat.Stamp(Line.StartMs);

    public string Who => Line.Speaker.Label;

    public string Text => Line.Text;

    public bool IsYou => Line.Speaker.IsYou;

    public override string ToString() => RecordDocument.LineLabel(Line);
}

/// <summary>x:Bind functions.</summary>
public static class Show
{
    public static Visibility If(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility Unless(bool hidden) => hidden ? Visibility.Collapsed : Visibility.Visible;
}

/// <summary>Elements the views build in code, styled from the app's tokens.</summary>
internal static class Parts
{
    public static Style TextStyle(string key) => (Style)Application.Current.Resources[key];

    /// <summary>A token brush in <paramref name="scope"/>'s theme (the views build again when the theme changes).</summary>
    public static Brush Brush(string key, FrameworkElement scope)
    {
        var resources = Application.Current.Resources;
        var theme = scope.ActualTheme == ElementTheme.Dark ? "Dark" : "Light";
        if (resources.ThemeDictionaries.TryGetValue(theme, out var dictionary)
            && dictionary is ResourceDictionary themed && themed.TryGetValue(key, out var brush) && brush is Brush found)
        {
            return found;
        }
        return (Brush)resources[key];
    }

    public static FontFamily Font(string key) => (FontFamily)Application.Current.Resources[key];

    public static TextBlock Text(string text, string style = "InkBodyStyle") => new() { Text = text, Style = TextStyle(style) };

    /// <summary>A section's small spaced capitals (the Mac's Paper.Eyebrow), a heading for Narrator.</summary>
    public static TextBlock Eyebrow(string text)
    {
        var eyebrow = Text(text.ToUpper(CultureInfo.CurrentCulture), "InkEyebrowStyle");
        AutomationProperties.SetHeadingLevel(eyebrow, Microsoft.UI.Xaml.Automation.Peers.AutomationHeadingLevel.Level3);
        AutomationProperties.SetName(eyebrow, text);
        return eyebrow;
    }

    /// <summary>A timestamp chip: plays the record from its moment.</summary>
    public static Button Chip(long ms, Action<long> play, FrameworkElement scope)
    {
        var chip = new Button
        {
            Content = new TextBlock
            {
                Text = LibraryFormat.Stamp(ms),
                FontFamily = Font("InkMonoFontFamily"),
                FontSize = 11,
                Foreground = Brush("InkThemBrush", scope),
            },
            Padding = new Thickness(5, 0, 5, 1),
            MinWidth = 0,
            MinHeight = 0,
            VerticalAlignment = VerticalAlignment.Center,
            Background = Brush("InkCardBrush", scope),
            BorderBrush = Brush("InkBorderBrush", scope),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(4),
        };
        AutomationProperties.SetName(chip, RecordDocument.ChipLabel(ms));
        ToolTipService.SetToolTip(chip, "Play from here");
        chip.Click += (_, _) => play(ms);
        return chip;
    }

    /// <summary>Words and a chip on one line, wrapping.</summary>
    public static Grid WithChip(UIElement words, long ms, Action<long> play, FrameworkElement scope)
    {
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var chip = Chip(ms, play, scope);
        chip.VerticalAlignment = VerticalAlignment.Top;
        Grid.SetColumn(chip, 1);
        row.Children.Add(words);
        row.Children.Add(chip);
        return row;
    }
}

/// <summary>Puts text on the clipboard.</summary>
internal static class TextClipboard
{
    public static void Copy(string text)
    {
        var package = new DataPackage();
        package.SetText(text);
        Clipboard.SetContent(package);
    }
}

/// <summary>
/// The search's pause (LibraryModel's ISearchScheduler): a one-shot DispatcherQueueTimer on the UI
/// thread, stopped by the next key. It never repeats, so nothing ticks while idle.
/// </summary>
public sealed class DispatcherSearchScheduler(DispatcherQueue queue) : ISearchScheduler
{
    public IDisposable After(TimeSpan delay, Action action)
    {
        ArgumentNullException.ThrowIfNull(action);
        var timer = queue.CreateTimer();
        timer.Interval = delay;
        timer.IsRepeating = false;
        timer.Tick += (sender, _) =>
        {
            sender.Stop();
            action();
        };
        timer.Start();
        return new Pending(timer);
    }

    private sealed class Pending(DispatcherQueueTimer timer) : IDisposable
    {
        public void Dispose() => timer.Stop();
    }
}
