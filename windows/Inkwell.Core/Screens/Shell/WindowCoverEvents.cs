// Which of Windows' events WindowCover listens to, and which of them make it measure again: pure, so
// the choice is tested; WindowCover makes the hooks.
//
// Uncovered, it hears what can cover the window: the front window changing, a move or resize ended,
// a window minimised or restored (a window moved over it by code is missed, and the window counts
// as uncovered, the side that costs nothing). Covered, it also hears only what uncovers it without
// any of those: a window destroyed, hidden or cloaked. Not a location change: one comes for every
// move of the mouse anywhere, a wake each even if dropped at once (rule 9), so a covering window
// moved or maximised by code (no drag, no hide, close or cloak, no change of the front window)
// leaves it covered until the next of these events. Destroy and hide also come for carets and the
// pointer, dropped at the first check: they come as things open, close and type, not as the mouse
// moves. They are hooked only while it is covered, only whole top-level windows count, and a burst
// (an app closing its windows) is measured once it settles, or half a second after it began.
// Hidden (in the tray), it hears nothing. Never the range between two events: the mouse's capture
// start and end lie in it, every press anywhere, and show lies between destroy and hide.
namespace Inkwell.Core.Screens;

public static class WindowCoverEvents
{
    private const uint SystemForeground = 0x0003;
    private const uint SystemMoveSizeEnd = 0x000B;
    private const uint SystemMinimizeStart = 0x0016;
    private const uint SystemMinimizeEnd = 0x0017;
    private const uint ObjectDestroy = 0x8001;
    private const uint ObjectHide = 0x8003;
    private const uint ObjectCloaked = 0x8017;
    /// <summary>EVENT_OBJECT_* begin here; the EVENT_SYSTEM_* come before.</summary>
    private const uint ObjectEvents = 0x8000;
    private const int ObjIdWindow = 0;
    private const int ChildIdSelf = 0;

    /// <summary>How long a burst of window events is let settle before one measure.</summary>
    public static readonly TimeSpan Settle = TimeSpan.FromMilliseconds(100);

    /// <summary>The longest a burst waits, however long it goes on.</summary>
    public static readonly TimeSpan MaxWait = TimeSpan.FromMilliseconds(500);

    private static readonly (uint Min, uint Max)[] Covering =
        [(SystemForeground, SystemForeground), (SystemMoveSizeEnd, SystemMoveSizeEnd), (SystemMinimizeStart, SystemMinimizeEnd)];

    private static readonly (uint Min, uint Max)[] Uncovering =
        [.. Covering, (ObjectDestroy, ObjectDestroy), (ObjectHide, ObjectHide), (ObjectCloaked, ObjectCloaked)];

    /// <summary>The event ranges to hook (each one SetWinEventHook) while the window is <paramref name="shown"/> and <paramref name="covered"/>.</summary>
    public static IReadOnlyList<(uint Min, uint Max)> Hooks(bool shown, bool covered) =>
        !shown ? [] : covered ? Uncovering : Covering;

    /// <summary>
    /// Whether an event makes it measure: a whole window (not a caret, the mouse or a part of one),
    /// and for the window events a top-level one (<paramref name="topLevel"/>: only those cover; a
    /// window already closed can't be asked, and counts as one).
    /// </summary>
    public static bool Measures(uint eventType, int idObject, int idChild, bool topLevel) =>
        idObject == ObjIdWindow && (eventType < ObjectEvents || (idChild == ChildIdSelf && topLevel));

    /// <summary>How long to wait, at a window event, before measuring: Settle, cut short so a burst that began <paramref name="sinceBurstBegan"/> ago waits no longer than MaxWait.</summary>
    public static TimeSpan SettleDelay(TimeSpan sinceBurstBegan)
    {
        var left = MaxWait - sinceBurstBegan;
        return left <= TimeSpan.Zero ? TimeSpan.Zero : left < Settle ? left : Settle;
    }

    /// <summary>Whether it waits for a burst to settle (the window events) rather than measuring at once.</summary>
    public static bool Settles(uint eventType) => eventType >= ObjectEvents;
}
