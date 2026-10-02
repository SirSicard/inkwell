// The consent step for a feature that sends the user's words to a language model (polish, voice
// edit, summaries and Ask), as a ContentDialog: the model's title, its message naming where the
// words go, Cancel, and the button that agrees. Only that button sends anything (ConsentModel.Allow);
// Cancel, Escape or the step closing leave the feature as it was (ConsentModel.Cancel).
//
// One ConsentDialog watches every consent a screen can ask for, because WinUI shows one
// ContentDialog at a time: it opens the step when a consent has one pending for its host, and
// closes it (as a Cancel) if the model closes the step, e.g. when the model moved while the user
// read. The first-run sheet is itself a ContentDialog, so it shows polish's step inline instead.
using System.Runtime.InteropServices;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed class ConsentDialog
{
    private readonly FrameworkElement owner;
    private readonly ConsentHost host;
    private readonly IReadOnlyList<ConsentModel> consents;
    private readonly ScreenLog log;
    private ContentDialog? open;
    private ConsentModel? openFor;

    /// <param name="owner">The element whose XamlRoot the dialog shows in; it watches while the owner is loaded.</param>
    public ConsentDialog(FrameworkElement owner, ConsentHost host, IReadOnlyList<ConsentModel> consents, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(owner);
        ArgumentNullException.ThrowIfNull(consents);
        this.owner = owner;
        this.host = host;
        this.consents = consents;
        this.log = log ?? ScreenLog.System;
        owner.Loaded += (_, _) => Watch(true);
        owner.Unloaded += (_, _) => Watch(false);
    }

    private void Watch(bool on)
    {
        foreach (var consent in consents)
        {
            consent.PropertyChanged -= OnChanged;
            if (on)
            {
                consent.PropertyChanged += OnChanged;
            }
        }
        if (on)
        {
            Update();
        }
        else
        {
            open?.Hide();
        }
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Update();

    private void Update()
    {
        if (open is not null)
        {
            if (openFor is not null && !openFor.IsShowingStep(host))
            {
                open.Hide();
            }
            return;
        }
        var next = consents.FirstOrDefault(c => c.IsShowingStep(host));
        if (next is not null && owner.XamlRoot is not null)
        {
            Show(next);
        }
    }

    private async void Show(ConsentModel consent)
    {
        if (consent.Pending is not ConsentDestination destination)
        {
            return;
        }
        var dialog = Build(consent.Feature, destination, owner.XamlRoot);
        open = dialog;
        openFor = consent;
        ContentDialogResult result;
        try
        {
            result = await dialog.ShowAsync();
        }
        catch (Exception e)
        {
            // Another dialog is up (or the dialog failed): nothing was agreed to, so nothing is sent.
            log.Write($"consent step could not be shown ({e.GetType().Name}); cancelled");
            open = null;
            openFor = null;
            consent.Cancel();
            return;
        }
        open = null;
        openFor = null;
        // Allow only for the destination the step named, and only while the model still asks.
        if (result == ContentDialogResult.Primary && consent.IsShowingStep(host) && consent.Pending == destination)
        {
            consent.Allow();
        }
        else if (consent.IsShowingStep(host))
        {
            consent.Cancel();
        }
        Update();
    }

    /// <summary>The step for <paramref name="feature"/> and <paramref name="destination"/>.</summary>
    public static ContentDialog Build(LlmFeature feature, ConsentDestination destination, XamlRoot root)
    {
        ArgumentNullException.ThrowIfNull(destination);
        var resources = Application.Current.Resources;
        var cancel = new Style(typeof(Button)) { BasedOn = (Style)resources["DefaultButtonStyle"] };
        cancel.Setters.Add(new Setter(AutomationProperties.NameProperty, ConsentModel.CancelName(feature)));
        var allow = new Style(typeof(Button)) { BasedOn = (Style)resources["InkAccentButtonStyle"] };
        allow.Setters.Add(new Setter(AutomationProperties.NameProperty, ConsentModel.AllowName(feature, destination)));
        return new ContentDialog
        {
            XamlRoot = root,
            // A dialog does not take the window's theme: the window's appearance, as shown now.
            RequestedTheme = (root?.Content as FrameworkElement)?.ActualTheme ?? ElementTheme.Default,
            Title = ConsentModel.Title(feature),
            Content = new TextBlock
            {
                Text = ConsentModel.Message(feature, destination),
                Style = (Style)resources["InkBodyStyle"],
            },
            PrimaryButtonText = ConsentModel.Button(feature, destination),
            PrimaryButtonStyle = allow,
            CloseButtonText = "Cancel",
            CloseButtonStyle = cancel,
            // Enter never agrees to send words off this PC: for a cloud model, focus and Enter land
            // on Cancel, and Allow is a deliberate press.
            DefaultButton = destination.IsOnDevice ? ContentDialogButton.Primary : ContentDialogButton.Close,
        };
    }
}
