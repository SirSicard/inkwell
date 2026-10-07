// The window's navigation follows the Router (Inkwell.Core/Screens/Shell/Router.cs): the items
// are its routes, grouped by section; selecting one opens it; Live comes and goes with the
// meeting. Screens are made once per route and kept, so a screen keeps its scroll and its
// half-typed text while another is shown. Owed carries the overdue count, Live a dot in their
// colour that pulses while motion is allowed. Nothing here redraws on its own: it changes only
// when the store, the router, the owed list or the appearance does.
//
// The window's keys (Windows has no menu bar; the title bar's "…" lists them under File and View):
// Ctrl+1–4 the routes, Ctrl+, Settings, Ctrl+F the search, Ctrl+Shift+R Record now, Ctrl+. Stop.
// While Live shows, Ctrl+1–4 and Ctrl+. are its own (the asks it answers, and Stop).
using Inkwell.Core;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media.Animation;
using VirtualKey = Windows.System.VirtualKey;
using VirtualKeyModifiers = Windows.System.VirtualKeyModifiers;

namespace Inkwell;

public sealed partial class MainWindow : Window
{
    /// <summary>The routes Ctrl+1–4 open, in order.</summary>
    private static readonly Route[] NumberedRoutes = [Route.Today, Route.Library, Route.Owed, Route.Live];

    private readonly Dictionary<Route, UIElement> screens = [];
    private readonly InfoBadge overdue = new() { Visibility = Visibility.Collapsed };
    private readonly InfoBadge liveDot = new() { Value = -1 };
    private readonly Storyboard pulse = new() { RepeatBehavior = RepeatBehavior.Forever, AutoReverse = true };
    private readonly List<KeyboardAccelerator> routeKeys = [];
    private KeyboardAccelerator? stopKey;
    private Router? router;
    private CoreStore? store;
    private MeetingModel? meetings;
    private OwedModel? owed;
    private GlowTheme? theme;
    private Func<Route, UIElement>? makeScreen;
    private Action<string>? search;
    private bool meetingLive;
    private bool syncing;
    private bool pulsing;
    private Route? shownRoute;
    private Microsoft.UI.Windowing.OverlappedPresenterState? frameState;

    public MainWindow()
    {
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBarArea);
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Inkwell.ico"));
        var fade = new DoubleAnimation { From = 1, To = 0.35, Duration = new Duration(TimeSpan.FromSeconds(0.9)) };
        Storyboard.SetTarget(fade, liveDot);
        Storyboard.SetTargetProperty(fade, "Opacity");
        pulse.Children.Add(fade);
        AutomationProperties.SetName(liveDot, "Recording");
        SystemMotion.Changed += UpdatePulse;
        Accelerators();
    }

    /// <summary>
    /// UI thread. Keeps the window inside its display's work area (WindowFrame.Fitted): moved and,
    /// if bigger, shrunk so all of it shows. Not while minimised or maximised (Windows places those).
    /// </summary>
    internal void FitToWorkArea()
    {
        if (AppWindow.Presenter is Microsoft.UI.Windowing.OverlappedPresenter { State: not Microsoft.UI.Windowing.OverlappedPresenterState.Restored }
            || TerraFX.Interop.Windows.Windows.IsIconic((TerraFX.Interop.Windows.HWND)Microsoft.UI.Win32Interop.GetWindowFromWindowId(AppWindow.Id)))
        {
            return;
        }
        var area = Microsoft.UI.Windowing.DisplayArea.GetFromWindowId(AppWindow.Id, Microsoft.UI.Windowing.DisplayAreaFallback.Nearest).WorkArea;
        var frame = new WindowFrame(AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height);
        var fitted = frame.Fitted(new WindowFrame(area.X, area.Y, area.Width, area.Height));
        if (fitted != frame)
        {
            AppWindow.MoveAndResize(new Windows.Graphics.RectInt32(fitted.X, fitted.Y, fitted.Width, fitted.Height));
        }
    }

    /// <summary>
    /// UI thread, on any change of the window (AppWindow.Changed). Restored from minimised or
    /// maximised, it comes back where it was, which may now be off screen: fitted then. Windows
    /// says "restored" while the window is still at its minimised place, so the fit waits until
    /// the restore is done (the next turn of the UI thread).
    /// </summary>
    internal void FrameChanged()
    {
        var state = (AppWindow.Presenter as Microsoft.UI.Windowing.OverlappedPresenter)?.State;
        if (state == Microsoft.UI.Windowing.OverlappedPresenterState.Restored
            && frameState is not null and not Microsoft.UI.Windowing.OverlappedPresenterState.Restored)
        {
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, FitToWorkArea);
        }
        frameState = state;
    }

    /// <summary>UI thread. The window's orb follows the shell's ink state; the edge glow follows the orb.</summary>
    internal void ShowInk(ShellInk ink)
    {
        // Soft blotting: the window's orb, wide behind the text, stops partway (the Drop blots fully).
        Orb.BlotDepth = 0.45;
        // It wanders, so it is not always in the same place (OrbWander).
        Orb.WanderBounds = OrbWander.Main;
        Orb.State = ink.State;
        ink.Changed += () =>
        {
            Orb.State = ink.State;
            DimOrb();
        };
        Orb.Drawn += Edge.Show;
    }

    /// <summary>UI thread, once. The window follows the appearance: its mode, the orb's colours, the edge.</summary>
    internal void ShowTheme(GlowTheme glow)
    {
        theme = glow;
        glow.Attach(Root, AppWindow);
        glow.Changed += ThemeChanged;
        ThemeChanged();
    }

    private void ThemeChanged()
    {
        if (theme is null)
        {
            return;
        }
        Orb.Look = theme.Look;
        Orb.AlwaysStill = theme.AlwaysStill;
        DimOrb();
        Edge.Set(theme.Colours, theme.EdgeGlow);
        liveDot.Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["GlowThemBrush"];
        UpdatePulse();
    }

    /// <summary>
    /// The orb behind the text: the user's strength at rest (70 % unless set), less while anything
    /// is live (dictating, a meeting, the final pass: 30 % at the default, OrbFade.BehindText), and
    /// no more than 45 % under High Contrast, so what is written over it reads.
    /// </summary>
    private void DimOrb() => Orb.OrbOpacity = OrbFade.BehindText(
        Orb.State.IsLive(), theme?.HighContrast == true, theme?.OrbStrength ?? OrbFade.RestBehindText);

    /// <summary>UI thread. The window was activated: a resting orb that has held its spot for a while moves (OrbWander.RestInterval).</summary>
    internal void WindowActivated() => Orb.Activated();

    /// <summary>UI thread, once. A milestone reached glows over the orb and says its line at the foot (MilestoneView).</summary>
    internal void ShowMilestones(StatsModel stats, WindowPresence presence)
    {
        if (theme is not null)
        {
            milestones = new MilestoneView(stats, presence, theme, Orb, GlowLayer, Root);
        }
    }

    /// <summary>Held for the window's life: it follows the stats model.</summary>
    private MilestoneView? milestones;

    /// <summary>UI thread. Other windows hid all of it, and now do not (WindowCover): a resting orb moves.</summary>
    internal void WindowUncovered() => Orb.Uncovered();

    /// <summary>
    /// UI thread, on any change of the window's place: on another monitor, a resting orb goes to a
    /// new spot.
    /// </summary>
    internal void PlaceChanged()
    {
        var display = Microsoft.UI.Windowing.DisplayArea.GetFromWindowId(AppWindow.Id, Microsoft.UI.Windowing.DisplayAreaFallback.Nearest).DisplayId.Value;
        if (lastDisplay is { } before && before != display)
        {
            Orb.MoveAtRest();
        }
        lastDisplay = display;
    }

    private ulong? lastDisplay;

    /// <summary>UI thread. Why the Drop cannot draw its ink (it shows a plain panel meanwhile), or null once it draws again.</summary>
    internal void ShowInkFailure(string? failure)
    {
        InkStatus.Text = failure is null ? "" : $"The Drop: {failure}";
        InkStatus.Visibility = failure is null ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>
    /// UI thread, once. Shows the store's routes: <paramref name="screen"/> makes a route's screen
    /// (once); <paramref name="meetingModel"/> records and stops from the keys and the "…" menu;
    /// <paramref name="owedModel"/> gives Owed its overdue count.
    /// </summary>
    /// <param name="searchSaid">Shows the Library's matches for a query typed in the search box.</param>
    public void Attach(
        CoreStore coreStore, Router shellRouter, Func<Route, UIElement> screen, Action<string> searchSaid, MeetingModel meetingModel,
        OwedModel owedModel)
    {
        search = searchSaid;
        store = coreStore;
        router = shellRouter;
        makeScreen = screen;
        meetings = meetingModel;
        owed = owedModel;
        meetingLive = store.Meeting is not null;
        BuildItems();
        store.PropertyChanged += (_, _) => StoreChanged();
        router.PropertyChanged += (_, _) => Show();
        owed.PropertyChanged += (_, _) => ShowOverdue();
        StoreChanged();
        ShowOverdue();
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
            UpdatePulse();
        }
    }

    /// <summary>The navigation's items for the routes listed now.</summary>
    private void BuildItems()
    {
        syncing = true;
        // The two badges move to the new items: a UI element has one parent.
        foreach (var old in Nav.MenuItems.Concat(Nav.FooterMenuItems).OfType<NavigationViewItem>())
        {
            old.InfoBadge = null;
        }
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
                    InfoBadge = route switch
                    {
                        Route.Owed => overdue,
                        Route.Live => liveDot,
                        _ => null,
                    },
                });
            }
        }
        syncing = false;
    }

    /// <summary>Owed's count: the promises overdue now (none: no badge).</summary>
    private void ShowOverdue()
    {
        if (owed is null)
        {
            return;
        }
        var count = owed.OverdueCount(DateTimeOffset.Now);
        overdue.Value = count;
        overdue.Visibility = count > 0 ? Visibility.Visible : Visibility.Collapsed;
        AutomationProperties.SetName(overdue, count == 1 ? "1 overdue" : $"{count} overdue");
    }

    /// <summary>Live's dot pulses while a meeting is live and motion is allowed (Animation effects on, not Always still).</summary>
    private void UpdatePulse()
    {
        var wanted = meetingLive && SystemMotion.AnimationsEnabled && theme?.AlwaysStill != true;
        if (wanted == pulsing)
        {
            return;
        }
        pulsing = wanted;
        if (wanted)
        {
            pulse.Begin();
        }
        else
        {
            pulse.Stop();
            liveDot.Opacity = 1;
        }
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

    /// <summary>The router's route: its item selected, its screen shown, the title and the keys it leaves to the screen.</summary>
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
        WindowTitle.Text = $"Inkwell · {route.Title()}";
        // Another screen in front of the orb: at rest it goes to a new spot (the Mac's contentID).
        if (shownRoute is { } before && before != route)
        {
            Orb.MoveAtRest();
        }
        shownRoute = route;
        // Live's own Ctrl+1–4 (the asks) and Ctrl+. (Stop) take over while it shows.
        foreach (var key in routeKeys)
        {
            key.IsEnabled = route != Route.Live;
        }
        if (stopKey is not null)
        {
            stopKey.IsEnabled = route != Route.Live;
        }
    }

    /// <summary>UI thread. The core's state: shown in the title bar only while it is not ready.</summary>
    public void ShowStatus(CoreStatus status)
    {
        Starting.IsActive = status.Kind == CoreStatusKind.Starting;
        Starting.Visibility = Starting.IsActive ? Visibility.Visible : Visibility.Collapsed;
        Status.Text = status.Kind switch
        {
            CoreStatusKind.Starting => "Starting the core",
            CoreStatusKind.Ready => "",
            CoreStatusKind.Failed => $"The core did not start: {status.Detail}",
            CoreStatusKind.MismatchedBuild => $"This shell and its core are from different builds (a {status.Detail} event did not decode)",
            CoreStatusKind.Stopped => "The core stopped",
            _ => Status.Text,
        };
        Status.Visibility = Status.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
    }

    // The keys and the "…" menu.

    private void Accelerators()
    {
        for (var i = 0; i < NumberedRoutes.Length; i++)
        {
            var route = NumberedRoutes[i];
            routeKeys.Add(Key(VirtualKey.Number1 + i, VirtualKeyModifiers.Control, () => OpenListed(route)));
        }
        Key(Comma, VirtualKeyModifiers.Control, () => router?.Open(Route.Settings));
        Key(VirtualKey.F, VirtualKeyModifiers.Control, FocusSearch);
        Key(VirtualKey.R, VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift, RecordNow);
        stopKey = Key(Period, VirtualKeyModifiers.Control, Stop);
    }

    private const VirtualKey Comma = (VirtualKey)188;
    private const VirtualKey Period = (VirtualKey)190;

    private KeyboardAccelerator Key(VirtualKey key, VirtualKeyModifiers modifiers, Action action)
    {
        var accelerator = new KeyboardAccelerator { Key = key, Modifiers = modifiers };
        accelerator.Invoked += (_, e) =>
        {
            e.Handled = true;
            action();
        };
        Root.KeyboardAccelerators.Add(accelerator);
        return accelerator;
    }

    /// <summary>Opens a route the navigation lists now (Live only while a meeting is live).</summary>
    private void OpenListed(Route route)
    {
        if (route.IsListed(meetingLive))
        {
            router?.Open(route);
        }
    }

    private void FocusSearch() => Search.Focus(FocusState.Keyboard);

    private void RecordNow()
    {
        if (store?.Meeting is null)
        {
            meetings?.RecordNow();
        }
    }

    private void Stop()
    {
        if (store?.Meeting is { Stopping: false })
        {
            meetings?.Stop();
        }
    }

    /// <summary>The "…" menu, made as it opens: File (Record Now or Stop) and View (the routes, Settings, Appearance).</summary>
    private void OnOverflowOpening(object? sender, object e)
    {
        OverflowMenu.Items.Clear();
        var file = new MenuFlyoutSubItem { Text = "File" };
        if (store?.Meeting is { } meeting)
        {
            file.Items.Add(Item("Stop Recording", "Ctrl+.", Stop, enabled: !meeting.Stopping));
        }
        else
        {
            file.Items.Add(Item("Record Now", "Ctrl+Shift+R", RecordNow, enabled: store?.Status.Kind == CoreStatusKind.Ready));
        }
        OverflowMenu.Items.Add(file);

        var view = new MenuFlyoutSubItem { Text = "View" };
        for (var i = 0; i < NumberedRoutes.Length; i++)
        {
            var route = NumberedRoutes[i];
            view.Items.Add(Item(route.Title(), $"Ctrl+{i + 1}", () => OpenListed(route), enabled: route.IsListed(meetingLive)));
        }
        view.Items.Add(Item("Settings", "Ctrl+,", () => router?.Open(Route.Settings)));
        view.Items.Add(Item("Find", "Ctrl+F", FocusSearch));
        view.Items.Add(new MenuFlyoutSeparator());
        if (theme is not null)
        {
            var appearance = new MenuFlyoutSubItem { Text = "Appearance" };
            foreach (var (mode, title) in new[] { (AppearanceMode.Light, "Light"), (AppearanceMode.Dark, "Dark"), (AppearanceMode.System, "Match System") })
            {
                var item = new RadioMenuFlyoutItem { Text = title, GroupName = "Appearance", IsChecked = theme.Appearance.Mode == mode };
                item.Click += (_, _) => theme.Appearance.SetMode(mode);
                appearance.Items.Add(item);
            }
            view.Items.Add(appearance);
        }
        OverflowMenu.Items.Add(view);
    }

    private static MenuFlyoutItem Item(string text, string keys, Action action, bool enabled = true)
    {
        var item = new MenuFlyoutItem { Text = text, KeyboardAcceleratorTextOverride = keys, IsEnabled = enabled };
        item.Click += (_, _) => action();
        return item;
    }
}
