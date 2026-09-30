// One core runs per process (ink_init refuses a second), so every test class that starts the real
// core joins this collection: xUnit runs its classes one at a time, and apart from the rest.
using Xunit;

namespace Inkwell.Core.Tests;

[CollectionDefinition(Name, DisableParallelization = true)]
public sealed class RealCore
{
    public const string Name = "the real core";
}
