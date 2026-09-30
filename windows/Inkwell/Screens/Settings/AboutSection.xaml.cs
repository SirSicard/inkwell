// Settings > About. The notices are static data: their rows are built once, and nothing there
// changes or ticks while the section is shown. The updates row follows its model (UpdatesModel),
// which changes only when the user presses its button. A model or component row with a text is an
// expander (Narrator reads its name and whether it is open); a model whose licence asks for no
// notice is a plain row.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class AboutSection : UserControl
{
    /// <param name="version">
    /// The version to show ("Inkwell 1.0.0"); null or blank for a development build (AboutModel).
    /// </param>
    public AboutSection(string? version)
        : this(new AboutModel(version))
    {
    }

    /// <param name="updates">The updates row's model; none: this copy does not update itself.</param>
    public AboutSection(AboutModel about, UpdatesModel? updates = null)
    {
        ArgumentNullException.ThrowIfNull(about);
        About = about;
        Updates = updates ?? new UpdatesModel(NoUpdater.Instance);
        InitializeComponent();
        Updates.PropertyChanged += (_, _) => RenderUpdates();
        RenderUpdates();
        RustList.ContainerContentChanging += OnRustRowChanging;
        foreach (var row in about.ModelRows)
        {
            ModelRows.Children.Add(Row(row));
        }
        foreach (var row in about.ComponentRows)
        {
            ComponentRows.Children.Add(Row(row));
        }
    }

    public AboutModel About { get; }

    public UpdatesModel Updates { get; }

    private void RenderUpdates()
    {
        UpdatesButton.Visibility = Updates.ActionTitle is null ? Visibility.Collapsed : Visibility.Visible;
        UpdatesButton.Content = Updates.ActionTitle;
        UpdatesButton.IsEnabled = Updates.CanAct;
        UpdatesLine.Text = Updates.Line ?? "";
        UpdatesLine.Visibility = Updates.Line is null ? Visibility.Collapsed : Visibility.Visible;
    }

    private async void OnUpdatesAct(object sender, RoutedEventArgs e)
    {
        try
        {
            await Updates.Act().ConfigureAwait(true);
        }
        catch (Exception failure)
        {
            // Act says its own failures on the row; anything else is logged by kind.
            ScreenLog.System.Write($"the updates row failed ({failure.GetType().Name})");
        }
    }

    /// <summary>A row: an expander opening onto its text, or, without a text, its title and line.</summary>
    private UIElement Row(NoticeRow row)
    {
        var label = new StackPanel { Spacing = 1, Padding = new Thickness(0, 6, 0, 6) };
        label.Children.Add(new TextBlock { Text = row.Title, Style = (Style)Resources["NoticeTitleStyle"] });
        label.Children.Add(new TextBlock { Text = row.Detail, Style = (Style)Application.Current.Resources["InkCaptionStyle"] });
        if (row.Text is null)
        {
            AutomationProperties.SetName(label, row.Title);
            AutomationProperties.SetHelpText(label, row.Detail);
            return label;
        }
        var expander = new Expander
        {
            Header = label,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Content = new ScrollViewer
            {
                MaxHeight = 220,
                Content = new TextBlock { Text = row.Text, Style = (Style)Resources["NoticeTextStyle"] },
            },
        };
        AutomationProperties.SetName(expander, row.Title);
        AutomationProperties.SetHelpText(expander, row.Detail);
        return expander;
    }

    /// <summary>
    /// A recycled crate row starts closed: the list reuses its containers as it scrolls, and an
    /// expander left open would otherwise show another crate's row open.
    /// </summary>
    private static void OnRustRowChanging(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        if (args.ItemContainer.ContentTemplateRoot is Expander expander)
        {
            expander.IsExpanded = false;
        }
    }
}
