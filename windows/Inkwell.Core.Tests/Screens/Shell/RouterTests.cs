// The routes the navigation lists (as the Mac's RouterTests; the window itself is checked by hand:
// windows/S3.3b-CHECKLIST.md).
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class RouterTests
{
    [Fact]
    public void EveryRouteHasATitleAndAGlyph()
    {
        foreach (var route in Routes.All)
        {
            Assert.False(string.IsNullOrEmpty(route.Title()), $"{route}");
            var glyph = Assert.Single(route.Glyph());
            Assert.InRange(glyph, '', ''); // the Segoe Fluent Icons private-use range
        }
    }

    [Fact]
    public void TheSidebarListsLiveOnlyWhileAMeetingIsLive()
    {
        var sections = Enum.GetValues<SidebarSection>();
        Assert.Equal([Route.Today, Route.Library, Route.Owed, Route.Stats, Route.Settings], sections.SelectMany(s => s.Listed(false)));
        Assert.Equal([Route.Today, Route.Library, Route.Owed, Route.Stats, Route.Live, Route.Settings], sections.SelectMany(s => s.Listed(true)));
        Assert.Empty(SidebarSection.Recording.Listed(false));
        Assert.Equal("While recording", SidebarSection.Recording.Title());
    }

    [Fact]
    public void AWindowShowingLiveGoesToTodayWhenTheMeetingEnds()
    {
        var router = new Router();
        Assert.Equal(Route.Today, router.Current);
        router.Open(Route.Live);
        router.Reconcile(meetingLive: true);
        Assert.Equal(Route.Live, router.Current);
        router.Reconcile(meetingLive: false);
        Assert.Equal(Route.Today, router.Selection);

        router.Open(Route.Library);
        router.Reconcile(meetingLive: false);
        Assert.Equal(Route.Library, router.Selection);

        router.Selection = null;
        Assert.Equal(Route.Today, router.Current);
    }
}
