// The player bar's code: it shows the RecordPlayer and sends it the user's presses. The playhead
// moves only while the player plays and the bar is loaded: a DispatcherQueueTimer runs then (30
// times a second, or once a second with animations off) and stops otherwise, so a paused or idle
// record draws nothing (architecture rule 9).
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Inkwell.Screens;

public sealed partial class PlayerBar : UserControl
{
    private readonly RecordPlayer _player;
    private readonly WaveformLoader _loader = new();
    private readonly WaveformView _wave;
    private readonly DispatcherQueueTimer _timer;
    private RecordDocument _document;
    private bool _loaded;

    /// <summary>The playhead moved (while playing): the record marks its line.</summary>
    public event EventHandler? Ticked;

    /// <param name="player">The open record's player (LibraryModel.Player).</param>
    /// <param name="document">The open record (LibraryModel.Document): its audio's caveats and waveform.</param>
    public PlayerBar(RecordPlayer player, RecordDocument document)
    {
        ArgumentNullException.ThrowIfNull(player);
        ArgumentNullException.ThrowIfNull(document);
        _player = player;
        _document = document;
        InitializeComponent();
        _wave = new WaveformView(ms => _player.Seek(ms));
        WaveHost.Child = _wave;
        YouSlider.Value = _player.YouVolume;
        ThemSlider.Value = _player.ThemVolume;
        YouSlider.IsEnabled = _player.Sides.Contains(Channel.Mic);
        ThemSlider.IsEnabled = _player.Sides.Contains(Channel.Far);
        _timer = DispatcherQueue.CreateTimer();
        _timer.IsRepeating = true;
        _timer.Tick += (_, _) => Tick();
        _player.PropertyChanged += (_, _) => PlayerChanged();
        _loader.PropertyChanged += (_, _) =>
        {
            _wave.SetWaveform(_loader.Waveform);
            ShowNotice();
        };
        Loaded += (_, _) =>
        {
            _loaded = true;
            _loader.Load(_document);
            PlayerChanged();
        };
        Unloaded += (_, _) =>
        {
            _loaded = false;
            _loader.Cancel();
            _timer.Stop();
        };
        PlayerChanged();
    }

    /// <summary>The record was read again (a note or a commitment changed): its caveats may have too.</summary>
    public void Update(RecordDocument document)
    {
        ArgumentNullException.ThrowIfNull(document);
        _document = document;
        ShowNotice();
    }

    private void PlayerChanged()
    {
        var playing = _player.IsPlaying;
        PlayIcon.Glyph = playing ? "" : "";
        AutomationProperties.SetName(PlayButton, _player.PlayLabel);
        ToolTipService.SetToolTip(PlayButton, _player.PlayLabel);
        ShowNotice();
        Tick();
        if (playing && _loaded)
        {
            // Animations off: the playhead steps once a second (the Mac's reduce motion).
            var animations = new Windows.UI.ViewManagement.UISettings().AnimationsEnabled;
            _timer.Interval = TimeSpan.FromSeconds(animations ? 1.0 / 30 : 1);
            if (!_timer.IsRunning)
            {
                _timer.Start();
            }
        }
        else
        {
            _timer.Stop();
        }
    }

    private void Tick()
    {
        var position = _player.PositionMs();
        _wave.SetPosition(position, _player.DurationMs);
        TimeText.Text = _player.TimeText(position);
        AutomationProperties.SetName(TimeText, _player.TimeLabel(position));
        Ticked?.Invoke(this, EventArgs.Empty);
    }

    private void ShowNotice()
    {
        var notice = _player.Notice(_document.PlaybackCaveats, _loader.Waveform.Partial);
        Notice.Text = notice ?? "";
        Notice.Visibility = notice is null ? Visibility.Collapsed : Visibility.Visible;
    }

    private void OnPlayClick(object sender, RoutedEventArgs e) => _player.Toggle();

    private void OnYouVolume(object sender, RangeBaseValueChangedEventArgs e) => _player.YouVolume = (float)e.NewValue;

    private void OnThemVolume(object sender, RangeBaseValueChangedEventArgs e) => _player.ThemVolume = (float)e.NewValue;
}
