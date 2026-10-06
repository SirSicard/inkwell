// Settings > Modes and the mode editor at the window's 720-epx minimum (and wider): how a row's
// name and controls share the row (ModesLayout, which the views' SettingColumnsPanel asks), and
// how its chips and buttons wrap (WrapLayout, the WrapPanel's), at the widths those windows give.
// The views themselves are WinUI's, laid out on a desktop; these are the choices they make.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class ModesLayoutTests
{
    /// <summary>The rows' buttons at Segoe UI 14 with the default button padding (measured on a desktop, rounded up).</summary>
    private static readonly (double Width, double Height)[] Buttons = [(64, 32), (68, 32), (92, 32)];

    /// <summary>Chips at 12 px with their padding: "Relaxed", "Clean up speech", "Polish · off".</summary>
    private static readonly (double Width, double Height)[] Chips = [(64, 20), (116, 20), (92, 20)];

    [Fact]
    public void AtTheWindowsMinimumARowPutsItsNameAboveItsControls()
    {
        var section = ModesLayout.SectionWidth(ModesLayout.WindowMinimum);
        Assert.Equal(186, section);
        Assert.True(ModesLayout.Stacks(section));
        var row = ModesLayout.Arrange(section, titleHeight: 20, controlsHeight: 100);
        Assert.True(row.Stacked);
        Assert.Equal((0d, 0d, section, 20d), row.Title);
        Assert.Equal((0d, 24d, section, 100d), row.Controls);
        Assert.Equal(124, row.Height);
        // Nothing in the controls is offered more than the row.
        Assert.Equal(section, ModesLayout.ControlsWidth(section));
    }

    [Theory]
    [InlineData(1040)]
    [InlineData(1280)]
    [InlineData(1920)]
    public void AWiderWindowPutsTheNameBesideItsControls(double window)
    {
        var section = ModesLayout.SectionWidth(window);
        Assert.False(ModesLayout.Stacks(section));
        var row = ModesLayout.Arrange(section, titleHeight: 20, controlsHeight: 100);
        Assert.False(row.Stacked);
        Assert.Equal(ModesLayout.TitleColumn + ModesLayout.ColumnSpacing, row.Controls.X);
        Assert.Equal(section, row.Controls.X + row.Controls.Width);
        Assert.True(row.Controls.Width >= ModesLayout.ControlsMinimum);
        Assert.Equal(100, row.Height);
    }

    /// <summary>The row switches once, at the width where the controls keep their minimum beside the name.</summary>
    [Fact]
    public void TheRowSwitchesOnceAsTheWindowWidens()
    {
        var switches = 0;
        var stacked = true;
        for (var window = ModesLayout.WindowMinimum; window <= 2000; window += 1)
        {
            var now = ModesLayout.Stacks(ModesLayout.SectionWidth(window));
            if (now != stacked)
            {
                switches++;
                stacked = now;
                Assert.Equal(ModesLayout.TitleColumn + ModesLayout.ColumnSpacing + ModesLayout.ControlsMinimum, ModesLayout.SectionWidth(window));
            }
        }
        Assert.Equal(1, switches);
    }

    /// <summary>Edit…, Delete and Confirm…, and the chips, wrap between items inside the row at 720 and stay on one line from the default 1040.</summary>
    [Theory]
    [InlineData(720, 2)]
    [InlineData(1040, 1)]
    public void TheButtonsAndChipsWrapInsideTheRow(double window, int buttonRows)
    {
        var controls = ModesLayout.ControlsWidth(ModesLayout.SectionWidth(window));
        foreach (var (items, rows) in new[] { (Buttons, buttonRows), (Chips, -1) })
        {
            var (places, width, _) = WrapLayout.Place(items, controls, spacing: 8, rowSpacing: 8);
            Assert.True(width <= controls, $"{width} past {controls} at {window}");
            for (var i = 0; i < items.Length; i++)
            {
                Assert.True(places[i].X + items[i].Width <= controls + 0.5, $"item {i} runs past the row at {window}");
            }
            if (rows > 0)
            {
                Assert.Equal(rows, places.Select(p => p.Y).Distinct().Count());
            }
        }
    }

    /// <summary>The editor: a dialog no wider than ContentDialog allows, inside the 720-epx window, its rows side by side (its name column is narrower).</summary>
    [Fact]
    public void TheEditorFitsTheWindowAndKeepsItsRowsSideBySide()
    {
        Assert.True(ModesLayout.DialogMaxWidth < ModesLayout.WindowMinimum);
        Assert.Equal(484, ModesLayout.EditorWidth);
        Assert.False(ModesLayout.Stacks(ModesLayout.EditorWidth, ModesLayout.EditorTitleColumn));
        var row = ModesLayout.Arrange(ModesLayout.EditorWidth, 20, 32, ModesLayout.EditorTitleColumn);
        Assert.Equal(ModesLayout.EditorWidth - ModesLayout.EditorTitleColumn - ModesLayout.ColumnSpacing, row.Controls.Width);
        Assert.True(row.Controls.Width >= ModesLayout.ControlsMinimum);
    }
}
