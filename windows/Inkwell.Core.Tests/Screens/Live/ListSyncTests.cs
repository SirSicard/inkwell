// The view's copy of the ledger follows the model's by the fewest changes (Windows only: the
// Mac's SwiftUI list diffs by id itself).
using System.Collections.ObjectModel;
using System.Collections.Specialized;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class ListSyncTests
{
    private static (ObservableCollection<string> List, List<NotifyCollectionChangedAction> Changes) Watched(params string[] items)
    {
        var list = new ObservableCollection<string>(items);
        var changes = new List<NotifyCollectionChangedAction>();
        list.CollectionChanged += (_, e) => changes.Add(e.Action);
        return (list, changes);
    }

    [Fact]
    public void ANewFinalReplacesOnlyTheWetTail()
    {
        var (list, changes) = Watched("a", "b", "wet");
        ListSync.Sync(list, ["a", "b", "c", "wet2"], string.Equals);
        Assert.Equal(["a", "b", "c", "wet2"], list);
        Assert.Equal([NotifyCollectionChangedAction.Remove, NotifyCollectionChangedAction.Add, NotifyCollectionChangedAction.Add], changes);
    }

    [Fact]
    public void LinesLetGoOfAtTheFrontAreRemovedThereAndTheRestKept()
    {
        var (list, changes) = Watched("a", "b", "c");
        ListSync.Sync(list, ["b", "c", "d"], string.Equals);
        Assert.Equal(["b", "c", "d"], list);
        Assert.Equal([NotifyCollectionChangedAction.Remove, NotifyCollectionChangedAction.Add], changes);
    }

    [Fact]
    public void AnythingElseStillEndsEqual()
    {
        var (list, _) = Watched("a", "b", "c");
        ListSync.Sync(list, ["x", "a"], string.Equals);
        Assert.Equal(["x", "a"], list);
        ListSync.Sync(list, [], string.Equals);
        Assert.Empty(list);
        ListSync.Sync(list, ["y"], string.Equals);
        Assert.Equal(["y"], list);
    }
}
