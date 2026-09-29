// Settings > AI. Each switch shows what the model says (on only with the core's consent and a
// working model), so switching one on reads back off while its consent step is up, and on once the
// core has recorded the consent. The section owns the Settings screen's ConsentDialog, for all
// three features (Voice's edit-key picker asks through it too): WinUI shows one dialog at a time.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class AiSection : UserControl
{
    private readonly AiSettings ai;
    private readonly PolishModel polish;
    private bool rendering;

    public AiSection(AiSettings ai, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        this.ai = ai;
        polish = ai.Polish;
        InitializeComponent();
        _ = new ConsentDialog(this, ConsentHost.Settings, [polish.Consent, ai.EditConsent, ai.MeetingsConsent], log);
        Loaded += (_, _) =>
        {
            ai.PropertyChanged += OnChanged;
            // Read again whenever Settings appears, as the Mac's does: a read that failed is
            // retried here (its line says so), and a switch changed elsewhere is current.
            polish.Load();
            ai.EditConsent.Load();
            ai.MeetingsConsent.Load();
            ai.Dictation.Load();
            Render();
        };
        Unloaded += (_, _) => ai.PropertyChanged -= OnChanged;
        Render();
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        rendering = true;
        try
        {
            Show(PolishSwitch, PolishStatus, polish.IsOn, polish.CanToggle, polish.Status, polish.IsProblem);
            Show(EditSwitch, EditStatus, ai.EditOn, ai.CanToggleEdit, ai.EditStatus, ai.EditIsProblem);
            Show(MeetingsSwitch, MeetingsStatus, ai.MeetingsAIOn, ai.CanToggleMeetingsAI, ai.MeetingsAIStatus, ai.MeetingsAIIsProblem);
        }
        finally
        {
            rendering = false;
        }
    }

    private static void Show(ToggleSwitch toggle, TextBlock line, bool on, bool enabled, string status, bool problem)
    {
        toggle.IsOn = on;
        toggle.IsEnabled = enabled;
        AutomationProperties.SetHelpText(toggle, status);
        line.Text = status;
        line.Style = (Style)Application.Current.Resources[problem ? "InkAlertTextStyle" : "InkCaptionStyle"];
    }

    private void OnPolishToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && PolishSwitch.IsOn != polish.IsOn)
        {
            polish.SetOn(PolishSwitch.IsOn);
            Render();
        }
    }

    private void OnEditToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && EditSwitch.IsOn != ai.EditOn)
        {
            ai.SetEdit(EditSwitch.IsOn);
            Render();
        }
    }

    private void OnMeetingsToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && MeetingsSwitch.IsOn != ai.MeetingsAIOn)
        {
            ai.SetMeetingsAI(MeetingsSwitch.IsOn);
            Render();
        }
    }
}
