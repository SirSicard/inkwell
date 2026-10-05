// Which of Windows' events WindowCover listens to, and which of them make it measure again: pure, so
// the choice is tested; WindowCover makes the hooks.
//
// Uncovered, it hears what can cover the window: the front window changing, a move or resize ended,
// a window minimised or restored (a window moved over it by code is missed, and the window counts
// as uncovered, the side that costs nothing). Covered, it also hears what can uncover it without
// any of those: a window destroyed, shown or hidden, moved or resized by code (a maximise without a
// drag), cloaked or uncloaked. Those come for every object on the desktop, a location change for
// each move of the mouse or the caret, so they are hooked only while it is covered, only whole
// top-level windows count, and a burst of them is measured once, when it settles. Hidden (in the
// tray), it hears nothing. Never the range between two events: the mouse's capture start and end
// lie in it, every press anywhere.
namespace Inkwell.Core.Screens;

public static class WindowCoverEvents
{
    private const uint SystemForeground = 0x0003;
    private const uint SystemMoveSizeEnd = 0x000B;
    private const uint SystemMinimizeStart = 0x0016;
    private const uint SystemMinimizeEnd = 0x0017;
    private const uint ObjectDestroy = 0x8001;
    private const uint ObjectHide = 0x8003;
    private const uint ObjectLocationChange = 0x800B;
    private const uint ObjectCloaked = 0x8017;
    private const uint ObjectUncloaked = 0x8018;
    /// <summary>EVENT_OBJECT_* begin here; the EVENT_SYSTEM_* come before.</summary>
    private const uint ObjectEvents = 0x8000;
    private const int ObjIdWindow = 0;
    private const int ChildIdSelf = 0;

    /// <summary>How long a burst of window events is let settle before one measure.</summary>
    public static readonly TimeSpan Settle = TimeSpan.FromMilliseconds(100);

    private static readonly (uint Min, uint Max)[] Covering =
        [(SystemForeground, SystemForeground), (SystemMoveSizeEnd, SystemMoveSizeEnd), (SystemMinimizeStart, SystemMinimizeEnd)];

    private static readonly (uint Min, uint Max)[] Uncovering =
        [.. Covering, (ObjectDestroy, ObjectHide), (ObjectLocationChange, ObjectLocationChange), (ObjectCloaked, ObjectUncloaked)];

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

    /// <summary>Whether it waits for a burst to settle (the window events) rather than measuring at once.</summary>
    public static bool Settles(uint eventType) => eventType >= ObjectEvents;
}
