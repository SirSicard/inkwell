// The Settings container. The coordinator passes the sections in the canvas's order (Permissions,
// Voice, AI, Modes, Snippets, Voice commands, Meetings, Models, Storage, About); this lays them
// out and keeps the list and the page in step. Nothing here runs while nobody scrolls or clicks.
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace Inkwell.Screens;

/// <summary>A Settings section: its name in the list, and its view (which starts with its heading).</summary>
public sealed record SettingsSectionEntry(string Title, UIElement Content);

public sealed partial class SettingsScreen : UserControl
{
    private readonly IReadOnlyList<SettingsSectionEntry> sections;
    /// <summary>The list was set from the page's scroll, not by the user: no jump.</summary>
    private bool syncing;
    /// <summary>A jump the list asked for is under way: the scroll it causes does not move the list.</summary>
    private bool jumping;

    public SettingsScreen(IReadOnlyList<SettingsSectionEntry> sections)
    {
        ArgumentNullException.ThrowIfNull(sections);
        this.sections = sections;
        InitializeComponent();
        foreach (var section in sections)
        {
            SectionList.Items.Add(section.Title);
            Sections.Children.Add(section.Content);
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
        jumping = true;
        sections[i].Content.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0, AnimationDesired = true });
    }

    private void OnScrolled(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        if (e.IsIntermediate)
        {
            return;
        }
        if (jumping)
        {
            jumping = false;
            return;
        }
        // The last section whose top has reached the top of the page (with a little slack).
        var current = 0;
        for (var i = 0; i < sections.Count; i++)
        {
            var top = sections[i].Content.TransformToVisual(Sections).TransformPoint(new Point(0, 0)).Y;
            if (top <= Scroller.VerticalOffset + 40)
            {
                current = i;
            }
        }
        if (SectionList.SelectedIndex != current)
        {
            syncing = true;
            SectionList.SelectedIndex = current;
            syncing = false;
        }
    }
}
