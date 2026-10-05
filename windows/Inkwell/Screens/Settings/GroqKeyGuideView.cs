// How to get a free Groq key (GroqKeyGuide, the Mac's GroqKeyGuide view): the cost line, its Rate
// Limits page a link, then the numbered steps, the first followed by its link to Groq's API Keys
// page. Links are inline, so a line wraps at any width. Narrator reads each step as "Step n of 4:"
// and its words (the number beside it is drawn only), and the keys link by what it does. The
// first run's sits over Groq's key box; Settings > AI's under the language model, in an Expander.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;

namespace Inkwell.Screens;

public sealed partial class GroqKeyGuideView : UserControl
{
    public GroqKeyGuideView(GroqKeyGuidePlace place)
    {
        var caption = (Style)Application.Current.Resources["InkCaptionStyle"];
        var panel = new StackPanel { Spacing = 4 };

        var cost = new TextBlock { Style = caption };
        var at = GroqKeyGuide.Cost.IndexOf(GroqKeyGuide.RateLimitsLink, StringComparison.Ordinal);
        cost.Inlines.Add(new Run { Text = GroqKeyGuide.Cost[..at] });
        cost.Inlines.Add(Link(GroqKeyGuide.RateLimitsLink, GroqKeyGuide.RateLimitsUrl, name: null));
        cost.Inlines.Add(new Run { Text = GroqKeyGuide.Cost[(at + GroqKeyGuide.RateLimitsLink.Length)..] });
        panel.Children.Add(cost);

        var steps = GroqKeyGuide.Steps(place);
        for (var i = 0; i < steps.Count; i++)
        {
            var row = new Grid { ColumnSpacing = 6 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var number = new TextBlock { Text = $"{i + 1}.", Style = caption };
            AutomationProperties.SetAccessibilityView(number, AccessibilityView.Raw);
            var text = new TextBlock { Style = caption };
            text.Inlines.Add(new Run { Text = steps[i] });
            if (i == 0)
            {
                text.Inlines.Add(new Run { Text = " " });
                text.Inlines.Add(Link(GroqKeyGuide.KeysLink, GroqKeyGuide.KeysUrl, GroqKeyGuide.KeysLinkName));
            }
            AutomationProperties.SetName(text, GroqKeyGuide.StepName(i + 1, steps.Count, steps[i]));
            Grid.SetColumn(text, 1);
            row.Children.Add(number);
            row.Children.Add(text);
            panel.Children.Add(row);
        }
        Content = panel;
    }

    /// <summary>A link that opens <paramref name="url"/> in the browser, named for Narrator when the words alone don't say what it does.</summary>
    private static Hyperlink Link(string text, string url, string? name)
    {
        var link = new Hyperlink { NavigateUri = new Uri(url) };
        link.Inlines.Add(new Run { Text = text });
        if (name is not null)
        {
            AutomationProperties.SetName(link, name);
        }
        return link;
    }
}
