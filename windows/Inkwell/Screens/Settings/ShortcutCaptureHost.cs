using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using VirtualKey = Windows.System.VirtualKey;

namespace Inkwell.Screens;

/// <summary>One preview input path for every shortcut picker; detached when capture ends.</summary>
internal sealed class ShortcutCaptureHost(ShortcutRecorderModel recorder)
{
    private UIElement? content;
    public void Capture(UIElement? root, bool on)
    {
        if (on && content is null && root is not null)
        {
            content = root;
            content.PreviewKeyDown += OnDown;
            content.PreviewKeyUp += OnUp;
        }
        else if (!on && content is not null)
        {
            content.PreviewKeyDown -= OnDown;
            content.PreviewKeyUp -= OnUp;
            content = null;
        }
    }
    private void OnDown(object sender, KeyRoutedEventArgs e) => e.Handled = recorder.Feed(new ShortcutCapture.Input.KeyDown(Key(e), e.KeyStatus.WasKeyDown));
    private void OnUp(object sender, KeyRoutedEventArgs e) => e.Handled = recorder.Feed(new ShortcutCapture.Input.KeyUp(Key(e)));
    private static CapturedKey Key(KeyRoutedEventArgs e)
    {
        var extended = e.KeyStatus.IsExtendedKey;
        return e.Key switch
        {
            VirtualKey.Control or VirtualKey.LeftControl or VirtualKey.RightControl =>
                CapturedKey.Side(e.Key == VirtualKey.RightControl || (e.Key == VirtualKey.Control && extended) ? "right_control" : "left_control"),
            VirtualKey.Menu or VirtualKey.LeftMenu or VirtualKey.RightMenu =>
                CapturedKey.Side(e.Key == VirtualKey.RightMenu || (e.Key == VirtualKey.Menu && extended) ? "right_alt" : "left_alt"),
            VirtualKey.Shift or VirtualKey.LeftShift or VirtualKey.RightShift =>
                CapturedKey.Side(e.Key == VirtualKey.RightShift || (e.Key == VirtualKey.Shift && e.KeyStatus.ScanCode == 0x36) ? "right_shift" : "left_shift"),
            VirtualKey.LeftWindows => CapturedKey.Side("left_win"),
            VirtualKey.RightWindows => CapturedKey.Side("right_win"),
            var key => CapturedKey.Of((uint)key),
        };
    }

}
