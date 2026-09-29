// The window's navigation follows the Router (Inkwell.Core/Screens/Shell/Router.cs): the items
// are its routes, grouped by section; selecting one opens it; Live comes and goes with the
// meeting. Screens are made once per route and kept, so a screen keeps its scroll and its
// half-typed text while another is shown. Nothing here redraws on its own: it changes only when
// the store or the router does.
using Inkwell.Core;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell;

public sealed partial class MainWindow : Window
{
    /// <summary>Today's ink zone and the rail elsewhere (the design's widths).</summary>
    private const double InkZoneWidth = 300;
    private const double RailWidth = 64;

    private readonly Dictionary<Route, UIElement> screens = [];
    private Router? router;
    private CoreStore? store;
    private Func<Route, UIElement>? makeScreen;
    private UIElement? todayFoot;
    private Action<string>? search;
    private bool meetingLive;
    private bool syncing;

    public MainWindow()
    {
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBarArea);
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Inkwell.ico"));
    }

    /// <summary>UI thread. The window's ink follows the shell's ink state.</summary>
    internal void ShowInk(ShellInk ink)
    {
        Ink.State = ink.State;
        ink.Changed += () => Ink.State = ink.State;
    }

    /// <summary>UI thread. Why the Drop cannot draw its ink (it shows a plain panel meanwhile), or null once it draws again.</summary>
    internal void ShowInkFailure(string? failure)
    {
        InkStatus.Text = failure is null ? "" : $"The Drop is showing a plain panel: {failure}";
        InkStatus.Visibility = failure is null ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>
    /// UI thread, once. Shows the store's routes: <paramref name="screen"/> makes a route's screen
    /// (once), <paramref name="railFoot"/> the controls at the foot of Today's ink zone.
    /// </summary>
    /// <param name="searchSaid">Shows the Library's matches for a query typed in the search box.</param>
    public void Attach(CoreStore coreStore, Router shellRouter, Func<Route, UIElement> screen, UIElement railFoot, Action<string> searchSaid)
    {
        search = searchSaid;
        store = coreStore;
        router = shellRouter;
        makeScreen = screen;
        todayFoot = railFoot;
        meetingLive = store.Meeting is not null;
        BuildItems();
        store.PropertyChanged += (_, _) => StoreChanged();
        router.PropertyChanged += (_, _) => Show();
        StoreChanged();
        Show();
    }

    private void StoreChanged()
    {
        if (store is null || router is null)
        {
            return;
        }
        ShowStatus(store.Status);
        var live = store.Meeting is not null;
        if (live != meetingLive)
        {
            meetingLive = live;
            BuildItems();
            router.Reconcile(live);
            Show();
        }
    }

    /// <summary>The navigation's items for the routes listed now.</summary>
    private void BuildItems()
    {
        syncing = true;
        Nav.MenuItems.Clear();
        Nav.FooterMenuItems.Clear();
        foreach (var section in Enum.GetValues<SidebarSection>())
        {
            var routes = section.Listed(meetingLive);
            if (routes.Count == 0)
            {
                continue;
            }
            var items = section == SidebarSection.App ? Nav.FooterMenuItems : Nav.MenuItems;
            if (section.Title() is string heading)
            {
                items.Add(new NavigationViewItemHeader { Content = heading });
            }
            foreach (var route in routes)
            {
                items.Add(new NavigationViewItem
                {
                    Content = route.Title(),
                    Tag = route,
                    Icon = new FontIcon { Glyph = route.Glyph() },
                });
            }
        }
        syncing = false;
    }

    private void OnSearchSubmitted(AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args) =>
        search?.Invoke(args.QueryText);

    private void OnNavigationSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (!syncing && args.SelectedItem is NavigationViewItem { Tag: Route route })
        {
            router?.Open(route);
        }
    }

    /// <summary>The router's route: its item selected, its screen shown, the rail's width.</summary>
    private void Show()
    {
        if (router is null || makeScreen is null)
        {
            return;
        }
        var route = router.Current;
        syncing = true;
        Nav.SelectedItem = Nav.MenuItems.Concat(Nav.FooterMenuItems)
            .OfType<NavigationViewItem>().FirstOrDefault(i => i.Tag is Route r && r == route);
        syncing = false;
        if (!screens.TryGetValue(route, out var screen))
        {
            screen = makeScreen(route);
            screens[route] = screen;
        }
        Screen.Content = screen;
        var wide = route == Route.Today;
        InkRail.Width = wide ? InkZoneWidth : RailWidth;
        // Today's zone knocks the wordmark out of the ink; the narrow rail does not.
        Ink.ShowsWordmark = wide;
        RailFoot.Content = wide ? todayFoot : null;
    }

    /// <summary>UI thread. The core's state: its status line, and its version once ready.</summary>
    public void ShowStatus(CoreStatus status)
    {
        Starting.IsActive = status.Kind == CoreStatusKind.Starting;
        Starting.Visibility = Starting.IsActive ? Visibility.Visible : Visibility.Collapsed;
        Status.Text = status.Kind switch
        {
            CoreStatusKind.Starting => "Starting the core",
            CoreStatusKind.Ready => "Ready",
            CoreStatusKind.Failed => $"The core did not start: {status.Detail}",
            CoreStatusKind.MismatchedBuild => $"This shell and its core are from different builds (a {status.Detail} event did not decode)",
            CoreStatusKind.Stopped => "The core stopped",
            _ => Status.Text,
        };
        Version.Text = status.Kind == CoreStatusKind.Ready ? $"core {status.Detail}" : "";
    }
}
