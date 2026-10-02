// The player's two lanes of bars (the Mac's WaveformView): them above the middle in their colour,
// you below in yours (the theme's), the played part solid and the rest faded, the playhead in the
// alert colour. Clicking or dragging moves the playhead; with focus, the arrow keys move it 10 s.
// Narrator reads it as "Waveform: you below, them above" with the time as its value.
//
// The bars are built once per waveform, size and theme; the playhead only moves a clip and a line,
// and only when the player bar's timer (which runs while playing) says so.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Windows.System;

namespace Inkwell.Screens;

public sealed partial class WaveformView : UserControl
{
    public const string AccessibleName = "Waveform: you below, them above";

    private readonly Canvas _canvas = new();
    private readonly Canvas _faded = new() { Opacity = 0.42 };
    private readonly Canvas _played = new();
    private readonly Rectangle _head = new() { Width = 2 };
    private readonly RectangleGeometry _clip = new();
    private readonly Action<long> _seek;
    private Waveform _waveform = Waveform.Empty;
    private long _position;
    private long _duration;
    private bool _dragging;

    public WaveformView(Action<long> seek)
    {
        _seek = seek;
        Height = 44;
        MinWidth = 80;
        IsTabStop = true;
        UseSystemFocusVisuals = true;
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        _played.Clip = _clip;
        _canvas.Children.Add(_faded);
        _canvas.Children.Add(_played);
        _canvas.Children.Add(_head);
        Content = _canvas;
        AutomationProperties.SetName(this, AccessibleName);
        SizeChanged += (_, _) => Draw();
        ActualThemeChanged += (_, _) => Draw();
        PointerPressed += OnPointerPressed;
        PointerMoved += OnPointerMoved;
        PointerReleased += OnPointerReleased;
        PointerCaptureLost += (_, _) => _dragging = false;
        KeyDown += OnKeyDown;
    }

    /// <summary>The playhead's time, as Narrator reads it.</summary>
    public string ValueText => LibraryFormat.Stamp(_position);

    public void SetWaveform(Waveform waveform)
    {
        _waveform = waveform;
        Draw();
    }

    /// <summary>Moves the playhead (the bars stay).</summary>
    public void SetPosition(long position, long duration)
    {
        _position = position;
        _duration = duration;
        PlaceHead();
    }

    private double Fraction => _duration > 0 ? Math.Clamp((double)_position / _duration, 0, 1) : 0;

    private void Draw()
    {
        _faded.Children.Clear();
        _played.Children.Clear();
        var width = ActualWidth;
        var height = ActualHeight > 0 ? ActualHeight : Height;
        if (width <= 0)
        {
            return;
        }
        // The lanes in the theme's colours (GlowTheme keeps the two brushes current).
        var them = (Brush)Application.Current.Resources["GlowThemBrush"];
        var you = (Brush)Application.Current.Resources["GlowYouBrush"];
        _head.Fill = Parts.Brush("InkAlertBrush", this);
        var bars = Math.Max(Math.Max(_waveform.You.Count, _waveform.Them.Count), 1);
        var step = width / bars;
        var lane = height / 2 - 2;
        var barWidth = Math.Max(Math.Min(2, step - 1), 1);
        for (var i = 0; i < bars; i++)
        {
            var x = i * step + step / 2 - barWidth / 2;
            var themHeight = Math.Max((i < _waveform.Them.Count ? _waveform.Them[i] : 0) * lane, 1);
            var youHeight = Math.Max((i < _waveform.You.Count ? _waveform.You[i] : 0) * lane, 1);
            foreach (var layer in (Canvas[])[_faded, _played])
            {
                layer.Children.Add(Bar(x, height / 2 - 1 - themHeight, barWidth, themHeight, them));
                layer.Children.Add(Bar(x, height / 2 + 1, barWidth, youHeight, you));
            }
        }
        _head.Height = height;
        PlaceHead();
    }

    private static Rectangle Bar(double x, double y, double width, double height, Brush fill)
    {
        var bar = new Rectangle { Width = width, Height = height, Fill = fill, RadiusX = 1, RadiusY = 1 };
        Canvas.SetLeft(bar, x);
        Canvas.SetTop(bar, y);
        return bar;
    }

    private void PlaceHead()
    {
        var width = ActualWidth;
        var height = ActualHeight > 0 ? ActualHeight : Height;
        var head = Fraction * width;
        _clip.Rect = new Rect(0, 0, head, height);
        Canvas.SetLeft(_head, head - 1);
    }

    private void SeekTo(double x)
    {
        var width = Math.Max(ActualWidth, 1);
        _seek((long)(Math.Clamp(x / width, 0, 1) * _duration));
    }

    private void OnPointerPressed(object sender, PointerRoutedEventArgs e)
    {
        _dragging = CapturePointer(e.Pointer);
        Focus(FocusState.Pointer);
        SeekTo(e.GetCurrentPoint(this).Position.X);
        e.Handled = true;
    }

    private void OnPointerMoved(object sender, PointerRoutedEventArgs e)
    {
        if (_dragging)
        {
            SeekTo(e.GetCurrentPoint(this).Position.X);
            e.Handled = true;
        }
    }

    private void OnPointerReleased(object sender, PointerRoutedEventArgs e)
    {
        _dragging = false;
        ReleasePointerCapture(e.Pointer);
    }

    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        switch (e.Key)
        {
            case VirtualKey.Right or VirtualKey.Up:
                _seek(Math.Min(_position + 10_000, _duration));
                e.Handled = true;
                break;
            case VirtualKey.Left or VirtualKey.Down:
                _seek(Math.Max(_position - 10_000, 0));
                e.Handled = true;
                break;
        }
    }

    protected override AutomationPeer OnCreateAutomationPeer() => new WaveformPeer(this);

    /// <summary>A slider that reads its time: the arrow keys move it.</summary>
    private sealed partial class WaveformPeer(WaveformView owner) : FrameworkElementAutomationPeer(owner), IValueProvider
    {
        public bool IsReadOnly => true;

        public string Value => owner.ValueText;

        public void SetValue(string value) => throw new InvalidOperationException("the waveform's time is set with the arrow keys");

        protected override object GetPatternCore(PatternInterface patternInterface) =>
            patternInterface == PatternInterface.Value ? this : base.GetPatternCore(patternInterface);

        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Slider;

        protected override string GetNameCore() => AccessibleName;

        protected override string GetClassNameCore() => nameof(WaveformView);

        protected override bool IsKeyboardFocusableCore() => true;
    }
}
