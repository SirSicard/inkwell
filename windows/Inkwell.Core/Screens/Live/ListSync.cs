// Brings a view's list in step with a model's new one by the fewest changes, so a list control
// keeps its rows (and its scroll) as the Live ledger grows: the oldest lines let go of at the front
// are removed there, the lines that stayed are kept, and the changed tail (a wet line replaced, new
// finals) is replaced. A final settled out of order changes a longer tail; still correct.
namespace Inkwell.Core.Screens;

public static class ListSync
{
    /// <summary>Makes <paramref name="target"/> hold <paramref name="source"/>, keeping the items <paramref name="same"/> says are unchanged.</summary>
    public static void Sync<T>(IList<T> target, IReadOnlyList<T> source, Func<T, T, bool> same)
    {
        ArgumentNullException.ThrowIfNull(target);
        ArgumentNullException.ThrowIfNull(source);
        ArgumentNullException.ThrowIfNull(same);
        // Let go of at the front: the first new item is further down the old list.
        if (target.Count > 0 && source.Count > 0 && !same(target[0], source[0]))
        {
            for (var k = 1; k < target.Count; k++)
            {
                if (same(target[k], source[0]))
                {
                    for (var i = 0; i < k; i++)
                    {
                        target.RemoveAt(0);
                    }
                    break;
                }
            }
        }
        var kept = 0;
        while (kept < target.Count && kept < source.Count && same(target[kept], source[kept]))
        {
            kept++;
        }
        for (var i = target.Count - 1; i >= kept; i--)
        {
            target.RemoveAt(i);
        }
        for (var i = kept; i < source.Count; i++)
        {
            target.Add(source[i]);
        }
    }
}
