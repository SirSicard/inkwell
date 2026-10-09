// The four permission cards (Settings and onboarding): what each one allows, whether it is on, and
// what to do when it is not. A port of the Mac's PermissionsModel.
//
// The core's probe answers three of them (permissions.check); the calendar is the shell's
// (ICalendarAccess). Nothing polls: a check runs when a screen showing the cards appears, when the
// app becomes active again (the user coming back from Windows Settings), after a request, and once
// after each launch. A check never prompts.
//
// Windows: only the microphone is a permission (three privacy switches in Settings > Privacy &
// security > Microphone). Its request opens that page, since Windows shows a desktop app no prompt.
// System audio (loopback), typing into other apps (SendInput and UI Automation) and the keyboard
// hook need none: the core answers them granted and cannot request them, so those cards read as
// allowed, say why, and never offer a button that could not work. The calendar is not reachable
// from this unpackaged build (NoCalendar): its card says it is not available, never refused.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A card.</summary>
public enum PermissionCard
{
    /// <summary>The microphone.</summary>
    HearYou,
    /// <summary>System audio: the far end.</summary>
    HearTheOthers,
    /// <summary>Typing into other apps, and the dictation key.</summary>
    TypeForYou,
    /// <summary>The calendar.</summary>
    KnowYourMeetings,
}

public static class PermissionCards
{
    /// <summary>The cards, in order.</summary>
    public static IReadOnlyList<PermissionCard> All { get; } =
        [PermissionCard.HearYou, PermissionCard.HearTheOthers, PermissionCard.TypeForYou, PermissionCard.KnowYourMeetings];

    /// <summary>The card's name, in the app's words.</summary>
    public static string Title(this PermissionCard card) => card switch
    {
        PermissionCard.HearYou => "Hear you",
        PermissionCard.HearTheOthers => "Hear the others",
        PermissionCard.TypeForYou => "Type for you",
        _ => "Know your meetings",
    };

    /// <summary>What it is and what it is for, when it is on.</summary>
    public static string Detail(this PermissionCard card) => card switch
    {
        PermissionCard.HearYou => "Microphone. Needed for dictation and your side of a call.",
        PermissionCard.HearTheOthers => "System audio. Needed for the other side of a call. Windows asks no permission for it.",
        PermissionCard.TypeForYou => "Typing into other apps. Needed to put dictated text where your cursor is, and for the dictation key. Windows asks no permission for it, but an app run as administrator can't be typed into.",
        _ => "Calendar. Names the call and who was in it.",
    };

    /// <summary>What is lost while it is off.</summary>
    public static string OffDetail(this PermissionCard card) => card switch
    {
        PermissionCard.HearYou => "The microphone is off. Nothing you say can be written down.",
        PermissionCard.HearTheOthers => "System audio is off. Meetings record only your voice.",
        PermissionCard.TypeForYou => "Typing into other apps is off. Dictation can't type into other apps or hear the dictation key.",
        _ => "The calendar is off. Meetings are named by the app they were in.",
    };

    /// <summary>What the calendar card says when this build cannot reach a calendar at all.</summary>
    public const string CalendarUnavailableDetail =
        "The calendar isn't available in this version of Inkwell for Windows. Meetings are named by the app they were in.";

    /// <summary>The card's line for <paramref name="state"/>.</summary>
    public static string Line(this PermissionCard card, CardState state) => state switch
    {
        CardState.Off => card.OffDetail(),
        CardState.Unavailable => CalendarUnavailableDetail,
        _ => card.Detail(),
    };

    /// <summary>The core's name for it; null for the calendar, which the shell reads itself.</summary>
    public static PermissionName? CorePermission(this PermissionCard card) => card switch
    {
        PermissionCard.HearYou => PermissionName.Microphone,
        PermissionCard.HearTheOthers => PermissionName.SystemAudio,
        PermissionCard.TypeForYou => PermissionName.Accessibility,
        _ => null,
    };

    /// <summary>Whether Windows has anything to ask for this card: the microphone (Settings) and a calendar this build can reach.</summary>
    public static bool CanRequest(this PermissionCard card, CardState state) => card switch
    {
        PermissionCard.HearYou => true,
        PermissionCard.KnowYourMeetings => state != CardState.Unavailable,
        _ => false,
    };

    /// <summary>The card's button, or null for none (allowed, checking, or nothing Windows can ask for).</summary>
    public static string? ActionTitle(this PermissionCard card, CardState state)
    {
        if (state is CardState.Allowed or CardState.Checking or CardState.Unavailable || !card.CanRequest(state))
        {
            return null;
        }
        // Windows never prompts a desktop app for the microphone: its button opens Settings.
        if (card == PermissionCard.HearYou)
        {
            return "Open Settings";
        }
        return state == CardState.Unknown ? "Open Settings" : "Allow";
    }

    /// <summary>A card's line when its request could not be carried out (Settings did not open).</summary>
    public const string RequestFailedLine =
        "Windows Settings didn't open. Open it from the Start menu: Privacy & security > Microphone.";

    /// <summary>What shows in place of a button.</summary>
    public static string StateLabel(CardState state) => state switch
    {
        CardState.Allowed => "Allowed",
        CardState.Checking => "Checking…",
        CardState.Unavailable => "Not available",
        CardState.Off => "Off",
        _ => "Can't be checked",
    };

    /// <summary>What a screen reader says for the card.</summary>
    public static string Spoken(this PermissionCard card, CardState state) => state switch
    {
        CardState.Allowed => "allowed",
        CardState.Off => $"off. {card.OffDetail()}",
        CardState.NotAsked => "not allowed yet",
        CardState.Unknown => "can't be checked",
        CardState.Unavailable => $"not available. {CalendarUnavailableDetail}",
        _ => "checking",
    };
}

/// <summary>The cards' states. UI thread; fed by the core's events.</summary>
public sealed class PermissionsModel : ObservableModel
{
    private readonly Dictionary<PermissionCard, CardState> states = [];
    private readonly Action<CoreCommand> send;
    private readonly ICalendarAccess calendar;
    /// <summary>Screens showing the cards now: activation re-checks only while one is up.</summary>
    private int visible;

    public PermissionsModel(Action<CoreCommand> send, ICalendarAccess? calendar = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.calendar = calendar ?? NoCalendar.Instance;
    }

    /// <summary>A check is on its way.</summary>
    public bool Checking { get; private set; }

    /// <summary>
    /// The card whose request the core could not carry out (Settings did not open), until the next
    /// request or the card reads allowed; null when none.
    /// </summary>
    public PermissionCard? RequestFailed { get; private set; }

    /// <summary>The card that asked last: a failed request names no card, and only one asks at a time.</summary>
    private PermissionCard? requested;

    public CardState State(PermissionCard card) => states.TryGetValue(card, out var state) ? state : CardState.Checking;

    /// <summary>Cards that are off, in order: what the sidebar and the needs-you banner warn about.</summary>
    public IReadOnlyList<PermissionCard> OffCards => PermissionCards.All.Where(c => State(c) == CardState.Off).ToList();

    /// <summary>Checks every card now.</summary>
    public void Refresh()
    {
        states[PermissionCard.KnowYourMeetings] = calendar.State();
        Checking = true;
        send(new CoreCommand.PermissionsCheck());
        Changed();
    }

    /// <summary>A screen with the cards appeared: check, and re-check on each activation while it is up.</summary>
    public void ScreenAppeared()
    {
        visible++;
        Refresh();
    }

    public void ScreenDisappeared() => visible = Math.Max(0, visible - 1);

    /// <summary>The app became active again: the user may be back from Windows Settings.</summary>
    public void AppBecameActive()
    {
        if (visible > 0)
        {
            Refresh();
        }
    }

    /// <summary>
    /// The user pressed the card's button. The microphone asks the core (which opens Settings >
    /// Privacy &amp; security > Microphone); the calendar asks the calendar. The other cards have
    /// nothing to ask on Windows, and do nothing.
    /// </summary>
    public void Request(PermissionCard card)
    {
        if (!card.CanRequest(State(card)))
        {
            return;
        }
        if (card.CorePermission() is PermissionName permission)
        {
            requested = card;
            if (RequestFailed is not null)
            {
                RequestFailed = null;
                Changed();
            }
            send(new CoreCommand.PermissionRequest(permission));
        }
        else
        {
            calendar.Request(Refresh);
        }
    }

    /// <summary>Whether this model shows <paramref name="failed"/>.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "permissions.check" or "permission.request";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case PermissionsChecked checkedEvent:
                Checking = false;
                states[PermissionCard.HearYou] = Card(checkedEvent.Microphone);
                states[PermissionCard.HearTheOthers] = Card(checkedEvent.SystemAudio);
                states[PermissionCard.TypeForYou] = Card(checkedEvent.Accessibility);
                if (RequestFailed is PermissionCard failedCard && State(failedCard) == CardState.Allowed)
                {
                    RequestFailed = null;
                }
                Changed();
                break;
            // The request did not happen (Settings did not open): said on the card that asked, not
            // only logged, so the button is not seen to do nothing.
            case CommandFailed { Command: "permission.request" }:
                RequestFailed = requested ?? PermissionCard.HearYou;
                Changed();
                break;
            // permission.requested needs nothing: the answer comes from Windows Settings, and the
            // check when the app is active again reads it.
            case CommandFailed { Command: "permissions.check" }:
                // No answer: what the cards showed before may no longer be true, and a card must
                // never read allowed (or off) on a guess.
                Checking = false;
                foreach (var card in PermissionCards.All.Where(c => c.CorePermission() is not null))
                {
                    states[card] = CardState.Unknown;
                }
                Changed();
                break;
            default:
                break;
        }
    }

    private static CardState Card(PermissionState state) => state switch
    {
        PermissionState.Granted => CardState.Allowed,
        PermissionState.Denied => CardState.Off,
        PermissionState.NotDetermined => CardState.NotAsked,
        _ => CardState.Unknown,
    };
}
