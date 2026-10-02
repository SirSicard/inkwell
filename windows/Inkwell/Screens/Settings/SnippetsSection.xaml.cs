// Settings > Snippets. Every change goes through the model, which sends the whole list and shows
// the core's answer; this only lays the rows out, keeps focus on the row a key press changed, and
// asks for an edit in a dialog.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class SnippetsSection : UserControl
{
    private IReadOnlyList<SnippetDraft>? shown;

    public SnippetsSection(SnippetsModel snippets)
    {
        Model = snippets ?? throw new ArgumentNullException(nameof(snippets));
        InitializeComponent();
        Model.PropertyChanged += (_, _) => Render();
        Render();
    }

    public SnippetsModel Model { get; }

    public static string StartOverTitle => PhraseLists.StartOverTitle;

    public static string StartOverHelp => PhraseLists.StartOverHelp;

    public static string FromImportText => PhraseLists.FromImportText;

    public static string EmptyText => SnippetsModel.EmptyText;

    public static string HelpText => SnippetsModel.HelpText;

    private void OnLoaded(object sender, RoutedEventArgs e) => Model.Load();

    private void Render()
    {
        RowsHost.IsEnabled = Model.Loaded;
        AddHost.IsEnabled = Model.Loaded;
        Empty.Visibility = Model.Loaded && Model.Rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (ReferenceEquals(shown, Model.Rows))
        {
            return;
        }
        shown = Model.Rows;
        var focus = RowFocus.Capture(Rows, XamlRoot);
        Rows.ItemsSource = Model.Rows;
        RowFocus.Restore(Rows, focus, item => ((SnippetDraft)item).Id);
    }

    private SnippetDraft? RowOf(object sender) => RowTag.Of(sender) is string id ? Model.Rows.FirstOrDefault(r => r.Id == id) : null;

    private void OnToggled(object sender, RoutedEventArgs e)
    {
        // Also raised when a row is laid out with its state: only the user's flip is a change.
        if (sender is ToggleSwitch toggle && RowOf(sender) is SnippetDraft row && row.Enabled != toggle.IsOn)
        {
            Model.SetEnabled(row.Id, toggle.IsOn);
        }
    }

    private void OnDelete(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is SnippetDraft row)
        {
            Model.Delete(row.Id);
        }
    }

    private async void OnEdit(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is not SnippetDraft row)
        {
            return;
        }
        var trigger = new TextBox { Header = "Trigger", Text = row.Trigger };
        var expansion = new TextBox { Header = "Text it becomes", Text = row.Expansion, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 160 };
        var category = new TextBox { Header = "Category (optional)", Text = row.Category };
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "Edit snippet",
            Content = new StackPanel { Spacing = 10, MinWidth = 360, Children = { trigger, expansion, category } },
            PrimaryButtonText = "Save",
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Primary,
        };
        trigger.TextChanged += (_, _) => dialog.IsPrimaryButtonEnabled = trigger.Text.Trim().Length > 0;
        try
        {
            if (await dialog.ShowAsync() == ContentDialogResult.Primary)
            {
                Model.Update(row with { Trigger = trigger.Text, Expansion = expansion.Text, Category = category.Text.Trim() });
            }
        }
        catch (Exception failure)
        {
            // Another dialog is open (only one can be): this one is not shown, and nothing changes.
            ScreenLog.System.Write($"the snippet editor could not open ({failure.GetType().Name})");
        }
    }

    private void OnNewTriggerChanged(object sender, TextChangedEventArgs e) => Add.IsEnabled = NewTrigger.Text.Trim().Length > 0;

    private void OnAdd(object sender, RoutedEventArgs e)
    {
        Model.Add(NewTrigger.Text, NewExpansion.Text, NewCategory.Text);
        NewTrigger.Text = "";
        NewExpansion.Text = "";
        NewCategory.Text = "";
    }

    private void OnStartOver(object sender, RoutedEventArgs e) => Model.StartOver();
}
