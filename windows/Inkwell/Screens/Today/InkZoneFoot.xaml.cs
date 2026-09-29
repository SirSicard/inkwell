// The ink zone's foot: the core's detection state, the key the core bound for dictation, and
// Record now (hidden while a meeting runs), with its failure under it. Listens only while loaded.
using System.ComponentModel;
using Inkwell.Core;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class InkZoneFoot : UserControl
{
    private readonly CoreStore store;
    private readonly RecordControlsModel controls;
    private readonly MeetingModel meetings;
    private bool loaded;

    public InkZoneFoot(CoreStore store, RecordControlsModel controls, MeetingModel meetings)
    {
        ArgumentNullException.ThrowIfNull(store);
        ArgumentNullException.ThrowIfNull(controls);
        ArgumentNullException.ThrowIfNull(meetings);
        this.store = store;
        this.controls = controls;
        this.meetings = meetings;
        InitializeComponent();
        AutomationProperties.SetHelpText(RecordNow, RecordControlsModel.RecordNowHint);
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    private INotifyPropertyChanged[] Models() => [store, controls, meetings];

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        loaded = true;
        foreach (var model in Models())
        {
            model.PropertyChanged += OnModelChanged;
        }
        Render();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        loaded = false;
        foreach (var model in Models())
        {
            model.PropertyChanged -= OnModelChanged;
        }
    }

    private void OnModelChanged(object? sender, PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        if (!loaded)
        {
            return;
        }
        var recording = store.Meeting is not null;
        ListeningLine.Text = RecordControlsModel.ListeningText(recording, store.Listening);
        DictateLine.Text = controls.DictateText ?? "";
        DictateLine.Visibility = controls.DictateText is null ? Visibility.Collapsed : Visibility.Visible;
        RecordNow.Visibility = recording ? Visibility.Collapsed : Visibility.Visible;
        var failure = recording ? null : meetings.FailureOn(MeetingPlace.RecordNow);
        RecordNowFailure.Text = failure ?? "";
        RecordNowFailure.Visibility = failure is null ? Visibility.Collapsed : Visibility.Visible;
    }

    private void OnRecordNow(object sender, RoutedEventArgs e) => meetings.RecordNow();
}
