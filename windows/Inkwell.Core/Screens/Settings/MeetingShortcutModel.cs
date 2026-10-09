using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>The confirmed global meeting toggle key. Failed saves never change the advertised key.</summary>
public sealed class MeetingShortcutModel(Action<CoreCommand> send) : ObservableModel
{
    public const string DefaultKey = "ctrl+shift+r";
    public string Key { get; private set; } = DefaultKey;
    public bool Loaded { get; private set; }
    public bool Active { get; private set; }
    public string? Problem { get; private set; }
    public string ShortcutLabel => Key == "off" ? "" : KeyNotation.Describe(Key).Cap;

    public void Load() => send(new CoreCommand.SettingGet(ShellSetting.MeetingsKey));

    public void SetKey(string key)
    {
        Problem = null;
        send(new CoreCommand.SettingSet(ShellSetting.MeetingsKey, key));
        Changed();
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case MeetingsShortcutState state:
                Key = state.Key;
                Loaded = true;
                Active = state.Active;
                Problem = state.Error;
                Changed();
                break;
            case CommandFailed failed when Handles(failed):
                Problem = $"Couldn't save the meeting shortcut: {failed.Message}. The previous key is unchanged.";
                Changed();
                break;
            case CoreStopped:
                Loaded = false;
                Active = false;
                Changed();
                break;
        }
    }

    public static bool Handles(CommandFailed failed) => failed.Id == ShellSetting.MeetingsKey.CommandId();
}
