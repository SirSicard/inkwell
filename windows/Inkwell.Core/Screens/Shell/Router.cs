// The window's destinations, as the Mac's Router: what the navigation lists, in which section,
// and which route is shown. Selecting an item is the whole navigation model: there is no second
// history to keep in step. The WinUI NavigationView renders it, so every item is a Narrator
// element with a working invoke.
namespace Inkwell.Core.Screens;

/// <summary>A navigation destination.</summary>
public enum Route
{
    Today,
    Library,
    Owed,
    Live,
    Settings,
}

/// <summary>The navigation's groups, in order.</summary>
public enum SidebarSection
{
    Main,
    Recording,
    App,
}

public static class Routes
{
    public static IReadOnlyList<Route> All { get; } = Enum.GetValues<Route>();

    /// <summary>The item's name, which Narrator reads.</summary>
    public static string Title(this Route route) => route switch
    {
        Route.Today => "Today",
        Route.Library => "Library",
        Route.Owed => "Owed",
        Route.Live => "Live",
        Route.Settings => "Settings",
        _ => throw new ArgumentOutOfRangeException(nameof(route)),
    };

    /// <summary>The item's Segoe Fluent Icons glyph (decorative: Narrator reads the title).</summary>
    public static string Glyph(this Route route) => route switch
    {
        Route.Today => "", // Brightness (a sun)
        Route.Library => "", // Library
        Route.Owed => "", // CheckboxComposite
        Route.Live => "", // (audio wave: Diagnostic)
        Route.Settings => "", // Setting
        _ => throw new ArgumentOutOfRangeException(nameof(route)),
    };

    public static SidebarSection Section(this Route route) => route switch
    {
        Route.Today or Route.Library or Route.Owed => SidebarSection.Main,
        Route.Live => SidebarSection.Recording,
        _ => SidebarSection.App,
    };

    /// <summary>Whether the navigation lists it now. Live exists only while a meeting is recorded or blotted.</summary>
    public static bool IsListed(this Route route, bool meetingLive) => route != Route.Live || meetingLive;

    /// <summary>The group's heading; null for none.</summary>
    public static string? Title(this SidebarSection section) => section == SidebarSection.Recording ? "While recording" : null;

    /// <summary>Its routes listed now, in declaration order.</summary>
    public static IReadOnlyList<Route> Listed(this SidebarSection section, bool meetingLive) =>
        All.Where(r => r.Section() == section && r.IsListed(meetingLive)).ToList();
}

/// <summary>Which screen the window shows. UI thread; the screens and the navigation share one.</summary>
public sealed class Router : ObservableModel
{
    private Route? selection = Route.Today;

    /// <summary>The selected route; null when a selection was cleared, and the window then shows Today.</summary>
    public Route? Selection
    {
        get => selection;
        set
        {
            if (selection != value)
            {
                selection = value;
                Changed();
            }
        }
    }

    /// <summary>The route whose screen is shown.</summary>
    public Route Current => selection ?? Route.Today;

    /// <summary>Shows <paramref name="route"/>.</summary>
    public void Open(Route route) => Selection = route;

    /// <summary>Keeps the selection on a listed item: when the meeting ends, Live leaves the navigation, and a window left showing it goes to Today.</summary>
    public void Reconcile(bool meetingLive)
    {
        if (selection is Route r && !r.IsListed(meetingLive))
        {
            Selection = Route.Today;
        }
    }
}
