// Settings > Meetings. The switches are the model's settings; a failed read or save is said under
// them. The model reads them at core.ready (the aggregator's), so nothing loads here.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class MeetingsSection : UserControl
{
    public MeetingsSection(MeetingModel meetings)
    {
        Model = meetings ?? throw new ArgumentNullException(nameof(meetings));
        InitializeComponent();
    }

    public MeetingModel Model { get; }

    public static string DetectDetail => "When an app opens the microphone for a call, Inkwell asks whether to record it. It never records without you saying so.";

    public static string HeadsetDetail => "With Bluetooth headphones, record their own microphone instead of the PC's. It carries only call-quality sound.";

    public static string SettingsFailedText => MeetingModel.SettingsFailedText;

    // Also raised when the model's value is shown: only the user's flip is a change.
    private void OnDetectToggled(object sender, RoutedEventArgs e)
    {
        if (Detect.IsOn != Model.Detect)
        {
            Model.SetDetect(Detect.IsOn);
        }
    }

    private void OnHeadsetToggled(object sender, RoutedEventArgs e)
    {
        if (HeadsetMic.IsOn != Model.HeadsetMic)
        {
            Model.SetHeadsetMic(HeadsetMic.IsOn);
        }
    }
}
