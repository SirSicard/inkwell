// What the shell shows about the core, from its events. Kept here, beside the session, so it is
// tested headless; the WinUI window only renders it.
using Inkwell.Core.Events;

namespace Inkwell.Core;

/// <summary>The core's state as the window shows it.</summary>
public enum CoreStatusKind
{
    /// <summary>ink_init returned; core.ready has not arrived.</summary>
    Starting,
    /// <summary>core.ready arrived with this shell's ABI.</summary>
    Ready,
    /// <summary>The core did not start, or started with another ABI.</summary>
    Failed,
    /// <summary>The core sent an event this build cannot read: shell and core do not match.</summary>
    MismatchedBuild,
    /// <summary>core.stopped arrived.</summary>
    Stopped,
}

/// <summary>
/// What the window shows. Detail: the version when ready, why when failed, the event's type when
/// the build is mismatched.
/// </summary>
public readonly record struct CoreStatus(CoreStatusKind Kind, string Detail = "")
{
    /// <summary>
    /// The status after <paramref name="evt"/>. What no screen shows is logged by name through
    /// <paramref name="log"/> (never an event's words): a command that failed, and an event this
    /// build cannot read (as the Mac's store does).
    /// </summary>
    public CoreStatus Next(InkEvent evt, Action<string> log)
    {
        ArgumentNullException.ThrowIfNull(evt);
        ArgumentNullException.ThrowIfNull(log);
        switch (evt)
        {
            case CoreReady ready when ready.Abi != InkSession.AbiVersion:
                return new CoreStatus(
                    CoreStatusKind.Failed, $"its ABI is {ready.Abi}, this shell's {InkSession.AbiVersion}");
            case CoreReady ready:
                return new CoreStatus(CoreStatusKind.Ready, ready.Version);
            case CoreStopped:
                return new CoreStatus(CoreStatusKind.Stopped);
            case UnknownEvent or UndecodableEvent:
                log($"mismatched build: a {(evt.Type.Length == 0 ? "typeless" : evt.Type)} event did not decode");
                return new CoreStatus(CoreStatusKind.MismatchedBuild, evt.Type);
            case CommandFailed failed:
                log($"command.failed for a {failed.Command} command; no screen shows it");
                return this;
            default:
                return this;
        }
    }
}
