// The Settings container. The coordinator passes the sections in the plan's order (General,
// Appearance, Permissions, Sound, Dictation, Modes, Snippets, Voice commands, AI, Meetings, Stats, Models,
// Storage, About: Screens.SettingsSections); this lays them out, each on its own card, and keeps
// the list and the page in step. Nothing here runs while nobody scrolls or clicks.
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace Inkwell.Screens;

/// <summary>A Settings section: its name in the list, and its view (which starts with its heading).</summary>
public sealed record SettingsSectionEntry(string Title, UIElement Content);

public sealed partial class SettingsScreen : UserControl
{
    private readonly IReadOnlyList<SettingsSectionEntry> sections;
    /// <summary>Each section's card, in the sections' order: what the list scrolls to and measures.</summary>
    private readonly List<Border> cards = [];
    /// <summary>The list was set from the page's scroll, not by the user: no jump.</summary>
    private bool syncing;
    /// <summary>
    /// The section a click scrolled to: it stays selected while any of it is in view, as the last
    /// sections cannot scroll to the top (and a click on one already in place scrolls nothing).
    /// </summary>
    private int? clicked;

    public SettingsScreen(IReadOnlyList<SettingsSectionEntry> sections)
    {
        ArgumentNullException.ThrowIfNull(sections);
        this.sections = sections;
        InitializeComponent();
        var cardStyle = (Style)Application.Current.Resources["InkCardStyle"];
        foreach (var section in sections)
        {
            SectionList.Items.Add(section.Title);
            var card = new Border { Style = cardStyle, Child = section.Content };
            cards.Add(card);
            Sections.Children.Add(card);
        }
        if (sections.Count > 0)
        {
            syncing = true;
            SectionList.SelectedIndex = 0;
            syncing = false;
        }
    }

    private void OnSectionChosen(object sender, SelectionChangedEventArgs e)
    {
        var i = SectionList.SelectedIndex;
        if (syncing || i < 0 || i >= sections.Count)
        {
            return;
        }
        clicked = i;
        cards[i].StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0, AnimationDesired = true });
    }

    private void OnScrolled(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        if (e.IsIntermediate)
        {
            return;
        }
        if (clicked is int chosen && chosen < cards.Count && InView(cards[chosen]))
        {
            return;
        }
        clicked = null;
        // At the end of the page the last section; else the last section whose top has reached
        // the top of the page (with a little slack).
        var current = 0;
        if (Scroller.VerticalOffset >= Scroller.ScrollableHeight - 1)
        {
            current = cards.Count - 1;
        }
        else
        {
            for (var i = 0; i < cards.Count; i++)
            {
                if (Top(cards[i]) <= Scroller.VerticalOffset + 40)
                {
                    current = i;
                }
            }
        }
        if (SectionList.SelectedIndex != current)
        {
            syncing = true;
            SectionList.SelectedIndex = current;
            syncing = false;
        }
    }

    /// <summary>Where a section's card starts on the page.</summary>
    private double Top(UIElement card) => card.TransformToVisual(Sections).TransformPoint(new Point(0, 0)).Y;

    /// <summary>Whether any of a section's card is in view.</summary>
    private bool InView(UIElement card)
    {
        var top = Top(card);
        return top + card.ActualSize.Y > Scroller.VerticalOffset && top < Scroller.VerticalOffset + Scroller.ViewportHeight;
    }
}
