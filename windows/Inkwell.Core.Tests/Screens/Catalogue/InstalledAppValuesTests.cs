// The registry values the Windows app directory reads (no Mac counterpart: the Mac asks NSWorkspace).
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class InstalledAppValuesTests
{
    [Fact]
    public void AnExeIsReadFromAppPathsAndDisplayIconValues()
    {
        Assert.Equal(@"C:\Apps\Example\example.exe", InstalledAppValues.ExePath(@"C:\Apps\Example\example.exe"));
        Assert.Equal(@"C:\Apps\Example\example.exe", InstalledAppValues.ExePath("\"C:\\Apps\\Example\\example.exe\""));
        Assert.Equal(@"C:\Apps\Example\Example.EXE", InstalledAppValues.ExePath(@"C:\Apps\Example\Example.EXE,0"));
        Assert.Equal(@"C:\Apps\Example\example.exe", InstalledAppValues.ExePath("\"C:\\Apps\\Example\\example.exe\",-101"));
        Assert.Equal(@"C:\Apps\Ex,ample\example.exe", InstalledAppValues.ExePath(@"C:\Apps\Ex,ample\example.exe"));
        Assert.Null(InstalledAppValues.ExePath(@"C:\Apps\Example\example.ico"));
        Assert.Null(InstalledAppValues.ExePath(@"C:\Windows\System32\shell32.dll,5"));
        Assert.Null(InstalledAppValues.ExePath("example.exe")); // not a path
        Assert.Null(InstalledAppValues.ExePath("  "));
        Assert.Null(InstalledAppValues.ExePath(null));
        Assert.Equal("example.exe", InstalledAppValues.FileName(@"C:\Apps\Example\example.exe"));
    }

    [Fact]
    public void AnAppGoesByItsInstalledNameNeverItsExe()
    {
        Assert.Equal("Example Writer", InstalledAppValues.Name("examplewriter.exe", "Example Writer", "EW", "EW Suite"));
        Assert.Equal("EW", InstalledAppValues.Name("examplewriter.exe", " ", "EW", "EW Suite"));
        Assert.Equal("EW Suite", InstalledAppValues.Name("examplewriter.exe", null, "ew.exe", "EW Suite"));
        Assert.Equal("Examplewriter", InstalledAppValues.Name("examplewriter.exe", null, null, null));
    }
}
