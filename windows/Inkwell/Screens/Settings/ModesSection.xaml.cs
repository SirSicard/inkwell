// Settings > Modes. Reads the modes each time the section appears; the rows are the model's, built
// here when they change (not on every change of the model: a row's button keeps the keyboard
// while the editor opens and closes). The editor, the delete confirmation, Confirm… and Start over
// are dialogs, shown one at a time as the model asks for them; only their agreeing buttons act.
using Inkwell.Core.Screens;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class ModesSection : UserControl
{
    private readonly Func<nint> windowHandle;
    private IReadOnlyList<ModeRow>? shownRows;
    private bool shownBusy;
    /// <summary>A dialog of this section is up (WinUI shows one ContentDialog at a time).</summary>
    private bool dialogOpen;

    public ModesSection(ModesModel modes, Func<nint>? windowHandle = null)
    {
        Model = modes ?? throw new ArgumentNullException(nameof(modes));
        this.windowHandle = windowHandle ?? (() => 0);
        InitializeComponent();
        Loaded += (_, _) =>
        {
            Model.PropertyChanged += OnChanged;
            Model.Load();
            Render();
        };
        Unloaded += (_, _) => Model.PropertyChanged -= OnChanged;
        Render();
    }

    public ModesModel Model { get; }

    public static string FailedText => ModesModel.FailedText;

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        Render();
        ShowDialogs();
    }

    private void OnTryAgain(object sender, RoutedEventArgs e) => Model.Load();

    private void OnStartOver(object sender, RoutedEventArgs e) => Model.AskStartOver();

    private void OnAdd(object sender, RoutedEventArgs e) => Model.Add();

    private void Render()
    {
        Failure.Visibility = Model.Failed ? Visibility.Visible : Visibility.Collapsed;
        ProblemText.Text = Model.Problem ?? "";
        ProblemText.Visibility = Model.Problem is null ? Visibility.Collapsed : Visibility.Visible;
        StartOverButton.Visibility = Model.Unreadable ? Visibility.Visible : Visibility.Collapsed;
        StartOverButton.IsEnabled = !Model.Busy;
        AddButton.IsEnabled = !Model.Failed;
        if (ReferenceEquals(shownRows, Model.Rows) && shownBusy == Model.Busy)
        {
            return;
        }
        shownRows = Model.Rows;
        shownBusy = Model.Busy;
        Rows.Children.Clear();
        foreach (var row in Model.Rows)
        {
            Rows.Children.Add(Row(row));
        }
    }

    private Style Local(string key) => (Style)Resources[key];

    private static Style App(string key) => (Style)Application.Current.Resources[key];

    /// <summary>One mode: its name | its chips, its apps, what stops its polish, and its buttons, each on a line of its own.</summary>
    private Border Row(ModeRow row)
    {
        var title = new TextBlock { Text = row.Title, Style = App("InkBodyStyle"), FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };
        var controls = new StackPanel { Spacing = 8 };

        var chips = new WrapPanel { Spacing = 6, RowSpacing = 6 };
        foreach (var trait in row.Traits)
        {
            chips.Children.Add(Chip(trait, muted: false, spoken: null, tip: null));
        }
        if (row.Polish.Chip is string polish)
        {
            chips.Children.Add(Chip(polish, row.Polish.ChipDimmed, row.Polish.SpokenChip, row.Polish.Why));
        }
        controls.Children.Add(chips);

        if (row.Apps.Count == 0)
        {
            controls.Children.Add(new TextBlock { Text = row.AppsText, Style = App("InkCaptionStyle") });
        }
        else
        {
            var tiles = new WrapPanel { Spacing = 6, RowSpacing = 6 };
            foreach (var app in row.Apps)
            {
                tiles.Children.Add(new AppIconTile { App = app });
            }
            AutomationProperties.SetAccessibilityView(tiles, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
            controls.Children.Add(tiles);
            var names = new TextBlock { Text = row.AppsText, Style = App("InkCaptionStyle") };
            AutomationProperties.SetName(names, $"Used in {row.AppsText}");
            controls.Children.Add(names);
        }

        if (row.Polish.RowNote is { } note)
        {
            controls.Children.Add(new TextBlock { Text = note.Text, Style = App("InkAlertTextStyle") });
        }

        var buttons = new WrapPanel { Spacing = 8, RowSpacing = 8 };
        buttons.Children.Add(Button("Edit…", $"Edit {row.Title}", () => Model.Edit(row.Id), enabled: true));
        if (!row.IsDefault)
        {
            buttons.Children.Add(Button("Delete", $"Delete {row.Title}", () => Model.AskDelete(row.Id), enabled: !Model.Busy));
        }
        switch (row.Polish.RowNote?.Fix)
        {
            case PolishFix.Confirm:
                buttons.Children.Add(Button("Confirm…", $"Confirm where {row.Title}'s model sends", () => Model.AskConfirm(row.Id), enabled: !Model.Busy));
                break;
            case PolishFix.Allow:
                buttons.Children.Add(Button("Allow…", $"Allow polish for {row.Title}", () => Model.AskConfirm(row.Id), enabled: !Model.Busy));
                break;
            default:
                break;
        }
        controls.Children.Add(buttons);

        var columns = new SettingColumnsPanel();
        columns.Children.Add(title);
        columns.Children.Add(controls);
        var border = new Border { Style = Local("ModeRowStyle"), Child = columns };
        AutomationProperties.SetName(border, row.Title);
        return border;
    }

    private Border Chip(string text, bool muted, string? spoken, string? tip)
    {
        var label = new TextBlock { Text = text, Style = Local(muted ? "ModeMutedChipTextStyle" : "ModeChipTextStyle") };
        if (spoken is not null)
        {
            AutomationProperties.SetName(label, spoken);
        }
        var chip = new Border { Style = Local(muted ? "ModeMutedChipStyle" : "ModeChipStyle"), Child = label };
        if (tip is not null)
        {
            ToolTipService.SetToolTip(chip, tip);
        }
        return chip;
    }

    private static Button Button(string content, string name, Action click, bool enabled)
    {
        var button = new Button { Content = content, IsEnabled = enabled };
        AutomationProperties.SetName(button, name);
        button.Click += (_, _) => click();
        return button;
    }

    /// <summary>The dialog the model asks for, if none of this section's is up.</summary>
    private void ShowDialogs()
    {
        if (dialogOpen || XamlRoot is null)
        {
            return;
        }
        if (Model.Editor is ModeEditor editor)
        {
            _ = Run(() => ModeEditorDialog.Show(Model, editor, XamlRoot, ActualTheme, windowHandle));
        }
        else if (Model.Deleting is ModeRow row)
        {
            _ = Run(() => ConfirmDelete(row));
        }
        else if (Model.Confirming is ModeConfirm confirm)
        {
            _ = Run(() => ConfirmModel(confirm));
        }
        else if (Model.ConfirmingStartOver)
        {
            _ = Run(ConfirmStartOver);
        }
    }

    private async Task Run(Func<Task> dialog)
    {
        dialogOpen = true;
        try
        {
            await dialog().ConfigureAwait(true);
        }
        catch (Exception e)
        {
            // Another dialog is open (only one can be): nothing was agreed to, so nothing is sent.
            ScreenLog.System.Write($"a modes dialog could not open ({e.GetType().Name})");
            if (Model.Editor is not null)
            {
                Model.CloseEditor();
            }
            if (Model.Deleting is not null)
            {
                Model.CancelDelete();
            }
            if (Model.Confirming is not null)
            {
                Model.CancelConfirm();
            }
            if (Model.ConfirmingStartOver)
            {
                Model.CancelStartOver();
            }
        }
        finally
        {
            dialogOpen = false;
        }
        // Another may have been asked for meanwhile.
        ShowDialogs();
    }

    private ContentDialog Dialog(string title, string message, string agree, bool cancelIsDefault) => new()
    {
        XamlRoot = XamlRoot,
        // A dialog does not take the window's theme: the appearance shown now.
        RequestedTheme = ActualTheme,
        Title = title,
        Content = new TextBlock { Text = message, TextWrapping = TextWrapping.Wrap, MaxWidth = 420, Style = App("InkBodyStyle") },
        PrimaryButtonText = agree,
        CloseButtonText = "Cancel",
        DefaultButton = cancelIsDefault ? ContentDialogButton.Close : ContentDialogButton.Primary,
    };

    /// <summary>Delete "Chat"?: where its apps go; Cancel is the default.</summary>
    private async Task ConfirmDelete(ModeRow row)
    {
        var dialog = Dialog(ModesModel.DeleteTitle(row), ModesModel.DeleteMessage(row), "Delete", cancelIsDefault: true);
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            Model.Delete(row);
        }
        else
        {
            Model.CancelDelete();
        }
    }

    /// <summary>
    /// A row's Confirm… or Allow…: where its model sends, as the consent step says it. For a model
    /// off this PC, Enter lands on Cancel, as the consent step's does.
    /// </summary>
    private async Task ConfirmModel(ModeConfirm confirm)
    {
        var dialog = Dialog(ModesModel.ConfirmTitle(confirm), Model.ConfirmMessage(confirm), ModesModel.ConfirmButton(confirm),
            cancelIsDefault: ConsentModel.FocusesCancel(confirm.Choice.Destination));
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            Model.ConfirmAllow(confirm);
        }
        else
        {
            Model.CancelConfirm();
        }
    }

    private async Task ConfirmStartOver()
    {
        var dialog = Dialog(ModesModel.StartOverTitle, ModesModel.StartOverMessage, "Start over", cancelIsDefault: true);
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            Model.StartOver();
        }
        else
        {
            Model.CancelStartOver();
        }
    }
}
