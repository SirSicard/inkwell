// The room Settings > Modes and the mode editor have, in effective pixels, and how a row's name and
// its controls share it: side by side where the row has room for both, else the name above the
// controls, each as wide as the row (the Mac's SettingColumnsLayout). The views' panel
// (SettingColumnsPanel) asks here, so the choice is tested headless, at the widths a 720-epx
// window gives.
namespace Inkwell.Core.Screens;

public static class ModesLayout
{
    /// <summary>The narrowest window Settings must fit: 720 epx, as the Mac's 720 pt.</summary>
    public const double WindowMinimum = 720;

    /// <summary>MainWindow's navigation pane (its OpenPaneLength).</summary>
    public const double NavigationPane = 220;

    /// <summary>Settings' list of sections (SettingsScreen's first column).</summary>
    public const double SectionList = 188;

    /// <summary>Settings' page padding, each side.</summary>
    public const double PagePadding = 40;

    /// <summary>A section card's padding and border, each side (InkCardStyle).</summary>
    public const double CardPadding = 22;
    public const double CardBorder = 1;

    /// <summary>A row's name column, side by side, and the gap after it.</summary>
    public const double TitleColumn = 150;
    public const double ColumnSpacing = 12;

    /// <summary>Between the name and the controls under it.</summary>
    public const double StackedSpacing = 4;

    /// <summary>The least room a row's controls get beside the name (the Mac's 240).</summary>
    public const double ControlsMinimum = 240;

    /// <summary>ContentDialog's widest (its ContentDialogMaxWidth) and its padding, each side.</summary>
    public const double DialogMaxWidth = 548;
    public const double DialogPadding = 24;

    /// <summary>The editor's name column, and the room its scroll bar keeps clear on the right.</summary>
    public const double EditorTitleColumn = 130;
    public const double EditorScrollGutter = 16;

    /// <summary>The room a section's rows have inside its card, in a window <paramref name="window"/> wide.</summary>
    public static double SectionWidth(double window) =>
        Math.Max(0, window - NavigationPane - SectionList - 2 * PagePadding - 2 * (CardPadding + CardBorder));

    /// <summary>The room the editor's rows have: the dialog at its widest, less its padding and the scroll bar's gutter.</summary>
    public static double EditorWidth => DialogMaxWidth - 2 * DialogPadding - EditorScrollGutter;

    /// <summary>Whether a row <paramref name="width"/> wide puts the name above its controls.</summary>
    public static bool Stacks(double width, double titleColumn = TitleColumn) =>
        double.IsFinite(width) && width < titleColumn + ColumnSpacing + ControlsMinimum;

    /// <summary>Where a row's name and controls go, and its size: side by side or stacked.</summary>
    /// <param name="width">The row's width (infinite: as wide as it likes, side by side).</param>
    /// <param name="titleHeight">The name's height at the width it gets.</param>
    /// <param name="controlsHeight">The controls' height at the width they get.</param>
    public static ColumnsArrangement Arrange(double width, double titleHeight, double controlsHeight, double titleColumn = TitleColumn)
    {
        if (Stacks(width, titleColumn))
        {
            var below = titleHeight + StackedSpacing;
            return new ColumnsArrangement(
                Stacked: true,
                Title: (0, 0, width, titleHeight),
                Controls: (0, below, width, controlsHeight),
                Height: below + controlsHeight);
        }
        var controlsX = titleColumn + ColumnSpacing;
        var controlsWidth = double.IsFinite(width) ? width - controlsX : double.PositiveInfinity;
        return new ColumnsArrangement(
            Stacked: false,
            Title: (0, 0, titleColumn, titleHeight),
            Controls: (controlsX, 0, controlsWidth, controlsHeight),
            Height: Math.Max(titleHeight, controlsHeight));
    }

    /// <summary>The width the controls get in a row <paramref name="width"/> wide.</summary>
    public static double ControlsWidth(double width, double titleColumn = TitleColumn) =>
        Stacks(width, titleColumn) ? width : width - titleColumn - ColumnSpacing;
}

/// <summary>A row's name and controls, placed: each as (X, Y, Width, Height) from the row's top left.</summary>
public readonly record struct ColumnsArrangement(
    bool Stacked,
    (double X, double Y, double Width, double Height) Title,
    (double X, double Y, double Width, double Height) Controls,
    double Height);
