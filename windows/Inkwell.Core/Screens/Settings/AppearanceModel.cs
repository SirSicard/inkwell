// Settings > Appearance, and what the whole shell reads to look the way the user chose: the mode,
// each mode's dot preset and own colours, the edge glow and the motion. Every value lives in the
// core's store (the appearance.* settings) and follows setting.value, so a change made anywhere
// (Settings, the first run, the title bar's menu) reaches every surface the same way. Until the
// core answers, and for any value it does not hold, the defaults: the system's mode, Indigo &
// Coral in both, the edge glowing, motion as Windows has it.
//
// A change shows at once and is sent; the core's setting.value confirms it. A read or a save
// that fails is said once in Settings ("Couldn't ..."), and a failed save is read again, so the
// screen shows what the store holds.
using Inkwell.Core.Events;
using Inkwell.Core.Glow;

namespace Inkwell.Core.Screens;

/// <summary>Light, dark, or whichever Windows uses for apps.</summary>
public enum AppearanceMode
{
    System,
    Light,
    Dark,
}

/// <summary>Whether the ink moves as Windows' Animation effects say, or always holds still.</summary>
public enum AppearanceMotion
{
    System,
    Still,
}

/// <summary>The appearance settings. UI thread only, like every screen model.</summary>
public sealed class AppearanceModel(Action<CoreCommand> send, ScreenLog? log = null) : ObservableModel
{
    public const string FailedText = "Couldn't read or save an appearance setting, so it may not be what it shows.";

    /// <summary>The preset's value for a colour: no colour of the user's own.</summary>
    public const string PresetValue = "preset";

    private static readonly ShellSetting[] Keys =
    [
        ShellSetting.AppearanceMode,
        ShellSetting.AppearanceDotsLight,
        ShellSetting.AppearanceDotsDark,
        ShellSetting.AppearanceYouLight,
        ShellSetting.AppearanceThemLight,
        ShellSetting.AppearanceYouDark,
        ShellSetting.AppearanceThemDark,
        ShellSetting.AppearanceEdgeGlow,
        ShellSetting.AppearanceMotion,
    ];

    /// <summary>The ids its setting commands carry.</summary>
    public static IReadOnlySet<string> SettingIds { get; } = Keys.Select(k => k.CommandId()).ToHashSet(StringComparer.Ordinal);

    private readonly Action<CoreCommand> send = send ?? throw new ArgumentNullException(nameof(send));
    private readonly ScreenLog log = log ?? ScreenLog.System;

    public AppearanceMode Mode { get; private set; } = AppearanceMode.System;

    /// <summary>The day mode's preset id.</summary>
    public string DotsLight { get; private set; } = GlowScheme.DefaultPreset;

    /// <summary>The night mode's preset id.</summary>
    public string DotsDark { get; private set; } = GlowScheme.DefaultPreset;

    public GlowRgb? YouLight { get; private set; }
    public GlowRgb? ThemLight { get; private set; }
    public GlowRgb? YouDark { get; private set; }
    public GlowRgb? ThemDark { get; private set; }

    /// <summary>Whether the window's edge glows while something is live.</summary>
    public bool EdgeGlow { get; private set; } = true;

    public AppearanceMotion Motion { get; private set; } = AppearanceMotion.System;

    /// <summary>A read or a save failed (Settings says <see cref="FailedText"/>).</summary>
    public bool Failed { get; private set; }

    /// <summary>Reads every appearance setting.</summary>
    public void Load()
    {
        foreach (var key in Keys)
        {
            send(new CoreCommand.SettingGet(key));
        }
    }

    /// <summary>Whether the shell is dark now, given whether Windows' app mode is.</summary>
    public bool IsDark(bool systemDark) => Mode switch
    {
        AppearanceMode.Light => false,
        AppearanceMode.Dark => true,
        _ => systemDark,
    };

    /// <summary>A mode's preset id.</summary>
    public string Dots(bool dark) => dark ? DotsDark : DotsLight;

    /// <summary>A mode's own colour for you or them, or null for the preset's.</summary>
    public GlowRgb? Custom(bool dark, bool you) => (dark, you) switch
    {
        (false, true) => YouLight,
        (false, false) => ThemLight,
        (true, true) => YouDark,
        _ => ThemDark,
    };

    /// <summary>The colours a mode resolves to.</summary>
    public GlowColours Colours(bool dark) => GlowScheme.Resolve(dark, Dots(dark), Custom(dark, you: true), Custom(dark, you: false));

    /// <summary>The colour the picker shows for you or them in a mode: the user's own, else the preset's (unfitted).</summary>
    public GlowRgb Chosen(bool dark, bool you)
    {
        var preset = GlowScheme.Preset(Dots(dark));
        return Custom(dark, you) ?? GlowRgb.From(you ? preset.You : preset.Them);
    }

    public void SetMode(AppearanceMode mode)
    {
        if (Mode == mode)
        {
            return;
        }
        Mode = mode;
        Set(ShellSetting.AppearanceMode, ModeValue(mode));
    }

    /// <summary>A mode's preset: it takes that mode's colours back from the user's own, as a preset is a full choice.</summary>
    public void SetDots(bool dark, string id)
    {
        if (!GlowScheme.IsPreset(id))
        {
            throw new ArgumentException($"no preset is called {id}", nameof(id));
        }
        if (dark)
        {
            DotsDark = id;
            YouDark = ThemDark = null;
        }
        else
        {
            DotsLight = id;
            YouLight = ThemLight = null;
        }
        Changed();
        send(new CoreCommand.SettingSet(dark ? ShellSetting.AppearanceDotsDark : ShellSetting.AppearanceDotsLight, id));
        send(new CoreCommand.SettingSet(ColourKey(dark, you: true), PresetValue));
        send(new CoreCommand.SettingSet(ColourKey(dark, you: false), PresetValue));
    }

    /// <summary>A mode's own colour for you or them; null goes back to the preset's.</summary>
    public void SetCustom(bool dark, bool you, GlowRgb? colour)
    {
        if (Custom(dark, you) == colour)
        {
            return;
        }
        SetColour(dark, you, colour);
        Set(ColourKey(dark, you), colour?.Hex ?? PresetValue);
    }

    public void SetEdgeGlow(bool on)
    {
        if (EdgeGlow == on)
        {
            return;
        }
        EdgeGlow = on;
        Set(ShellSetting.AppearanceEdgeGlow, on ? "on" : "off");
    }

    public void SetMotion(AppearanceMotion motion)
    {
        if (Motion == motion)
        {
            return;
        }
        Motion = motion;
        Set(ShellSetting.AppearanceMotion, motion == AppearanceMotion.Still ? "still" : "system");
    }

    /// <summary>Whether this model shows the failure: a read or a save of an appearance setting.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "setting.get" or "setting.set" && failed.Id is string id && SettingIds.Contains(id);
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case SettingValue value when value.Key.StartsWith("appearance.", StringComparison.Ordinal):
                if (Take(value.Key, value.Value))
                {
                    Changed();
                }
                break;
            case CommandFailed failed when Handles(failed):
                // The kind only: the core's message names the key, never anything the user said.
                log.Write($"{failed.Command} of an appearance setting failed");
                Failed = true;
                foreach (var key in Keys.Where(k => failed.Command == "setting.set" && k.CommandId() == failed.Id))
                {
                    // Show what the store holds, not what was asked for.
                    send(new CoreCommand.SettingGet(key));
                }
                Changed();
                break;
            default:
                break;
        }
    }

    /// <summary>A value from the store; anything this shell cannot read is the default. True when it changed something.</summary>
    private bool Take(string key, string? value)
    {
        var before = (Mode, DotsLight, DotsDark, YouLight, ThemLight, YouDark, ThemDark, EdgeGlow, Motion);
        switch (key)
        {
            case "appearance.mode":
                Mode = value switch
                {
                    "light" => AppearanceMode.Light,
                    "dark" => AppearanceMode.Dark,
                    _ => AppearanceMode.System,
                };
                break;
            case "appearance.dots.light":
                DotsLight = GlowScheme.IsPreset(value) ? value! : GlowScheme.DefaultPreset;
                break;
            case "appearance.dots.dark":
                DotsDark = GlowScheme.IsPreset(value) ? value! : GlowScheme.DefaultPreset;
                break;
            case "appearance.you.light":
                YouLight = GlowRgb.Parse(value);
                break;
            case "appearance.them.light":
                ThemLight = GlowRgb.Parse(value);
                break;
            case "appearance.you.dark":
                YouDark = GlowRgb.Parse(value);
                break;
            case "appearance.them.dark":
                ThemDark = GlowRgb.Parse(value);
                break;
            case "appearance.edge_glow":
                EdgeGlow = value != "off";
                break;
            case "appearance.motion":
                Motion = value == "still" ? AppearanceMotion.Still : AppearanceMotion.System;
                break;
            default:
                return false;
        }
        return before != (Mode, DotsLight, DotsDark, YouLight, ThemLight, YouDark, ThemDark, EdgeGlow, Motion);
    }

    private void SetColour(bool dark, bool you, GlowRgb? colour)
    {
        switch ((dark, you))
        {
            case (false, true):
                YouLight = colour;
                break;
            case (false, false):
                ThemLight = colour;
                break;
            case (true, true):
                YouDark = colour;
                break;
            default:
                ThemDark = colour;
                break;
        }
    }

    private void Set(ShellSetting key, string value)
    {
        Changed();
        send(new CoreCommand.SettingSet(key, value));
    }

    private static ShellSetting ColourKey(bool dark, bool you) => (dark, you) switch
    {
        (false, true) => ShellSetting.AppearanceYouLight,
        (false, false) => ShellSetting.AppearanceThemLight,
        (true, true) => ShellSetting.AppearanceYouDark,
        _ => ShellSetting.AppearanceThemDark,
    };

    /// <summary>The mode's stored value.</summary>
    public static string ModeValue(AppearanceMode mode) => mode switch
    {
        AppearanceMode.Light => "light",
        AppearanceMode.Dark => "dark",
        _ => "system",
    };
}
