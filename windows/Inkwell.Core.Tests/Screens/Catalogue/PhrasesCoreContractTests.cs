// Against the real core (as the Mac's PhrasesCoreContractTests): the lists' and the note's commands
// are read and answered with events this shell decodes. Needs ink_ffi.dll (as SmokeTests), so it
// runs on the PC, not the Mac's fast loop.
using Inkwell.Core.Events;
using Inkwell.Core.Native;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class PhrasesCoreContractTests
{
    /// <summary>
    /// One core per process: another test class's session may be running, so a start that finds
    /// one waits for it to end (up to a minute) rather than failing.
    /// </summary>
    private static InkSession Start(InkConfig config, Action<InkEvent> onEvent)
    {
        var until = DateTime.UtcNow + TimeSpan.FromMinutes(1);
        while (true)
        {
            try
            {
                return InkSession.Start(config, onEvent);
            }
            catch (InkStatusException e) when (e.Code == InkStatus.AlreadyInitialized && DateTime.UtcNow < until)
            {
                Thread.Sleep(100);
            }
        }
    }

    [Fact]
    public void TheListsAndTheNoteAreReadByTheCoreAndAnswered()
    {
        var data = Path.Combine(Path.GetTempPath(), $"inkwell-phrases-{Guid.NewGuid():N}");
        var events = new Events();
        var session = Start(new InkConfig(data, LogLevel: "warn"), events.Record);
        try
        {
            T? Answer<T>(CoreCommand command, Func<T, bool>? pick = null) where T : InkEvent
            {
                var before = events.All.Count;
                session.Command(command.Json);
                var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
                while (DateTime.UtcNow < until)
                {
                    if (events.All.Skip(before).OfType<T>().FirstOrDefault(e => pick?.Invoke(e) ?? true) is T found)
                    {
                        return found;
                    }
                    Thread.Sleep(10);
                }
                return null;
            }

            var empty = Answer<SnippetsListed>(new CoreCommand.SnippetsList("snippets:1"));
            Assert.NotNull(empty);
            Assert.Empty(empty.Snippets);
            Assert.False(empty.FromImport);
            var saved = Answer<SnippetsListed>(
                new CoreCommand.SnippetsSave([new SnippetDraft("a", "brb", "be right back")], false, "snippets:2"), l => l.Ref == "snippets:2");
            Assert.Equal("be right back", saved?.Snippets[0].Expansion);
            var defaults = Answer<VoiceCommandsListed>(new CoreCommand.VoiceCommandsList("voice_commands:1"));
            Assert.NotNull(defaults);
            Assert.False(defaults.Enabled); // off until the user turns them on
            Assert.NotEmpty(defaults.Commands);
            var commands = Answer<VoiceCommandsListed>(
                new CoreCommand.VoiceCommandsSave(true, "inkwell", [new VoiceCommandDraft("t", ["sign off"], CommandAction.InsertText, "Best")], false, "voice_commands:2"),
                l => l.Ref == "voice_commands:2");
            Assert.True(commands?.Commands[0].CarriedOut);
            var notes = Answer<ImportNotes>(new CoreCommand.ImportNotes());
            Assert.NotNull(notes);
            Assert.Null(notes.Key); // no import, nothing to say
            var dismissed = Answer<SettingValue>(new CoreCommand.SettingSet(ShellSetting.ImportKeyNote, "dismissed"));
            Assert.Equal("dismissed", dismissed?.Value);
            Assert.DoesNotContain(events.All, e => e is UndecodableEvent or UnknownEvent);
        }
        finally
        {
            session.Shutdown();
            if (Directory.Exists(data))
            {
                Directory.Delete(data, recursive: true);
            }
        }
    }
}
