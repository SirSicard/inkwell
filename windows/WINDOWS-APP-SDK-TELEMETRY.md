# What the Windows App SDK sends, in an Inkwell build

A finding, read from Microsoft's own statements and the SDK's public source on 2026-09-30. No
network or ETW capture was taken: what is marked *not verified* below is what one would settle.

Inkwell itself sends nothing about the PC or its use (README, "Privacy"). The Windows app ships
Microsoft's Windows App SDK runtime beside the exe (WinUI 3 and the runtime it needs,
self-contained, nothing installed system-wide). Its licence says the software "may collect
information about you and your use of the software, and send that to Microsoft", that some of it
can be opted out of "but not all, as described in the product documentation", and that apps which
use its data-collection features must give their users notice (section 2(a) of the Microsoft
Software License Terms, in Settings > About).

## How it collects

- **Through Windows, not on its own.** Microsoft's answer on the SDK's repository: "The runtime
  components of the Windows App SDK are an extension of Windows and follow Windows policies and
  practices for telemetry and data collection." The SDK writes TraceLogging events (ETW) under
  Microsoft's telemetry keywords; Windows' own diagnostic data service (Connected User Experiences
  and Telemetry) decides whether they leave the PC, under the same setting as Windows' own events.
  The SDK opens no network connection of its own for this.
- **What an event carries.** In the SDK's public source (`WindowsAppRuntimeInsights.h`, and each
  feature's `*Telemetry.h`): the SDK's version and channel, whether the process is packaged,
  self-contained or debugged, and the event's own fields, tagged by privacy data type (for example
  product and service usage, performance). Where an event names the app, an unpackaged app is named
  by its executable's file name with the folder removed (`TelemetryHelper.h`): here, `Inkwell.exe`.
  Nothing Inkwell handles (audio, transcripts, notes, settings) passes through these APIs.
- **Which features.** The events are written by the SDK features that are called. Inkwell calls
  WinUI 3 (windows, controls, composition) and the self-contained runtime's start-up, and none of
  the features that carry their own telemetry and data-collection hooks: no app lifecycle or
  activation APIs, notifications, push, OAuth, storage pickers or WebView2 (Inkwell creates no web
  view, so the WebView2 runtime is never started). Their DLLs ship because the self-contained
  runtime ships whole. *Not verified:* which events WinUI's closed-source part writes during an
  ordinary session.
- **Builds from source send nothing.** Microsoft: telemetry is "injected in our internal build /
  release pipelines", so a copy built from the public source "won't send telemetry to Microsoft".
  That covers the Foundation parts only: WinUI 3 is not built from public source, so Inkwell ships
  Microsoft's binaries.

## What can be turned off, and by whom

- **The user, in Windows.** The same switch as for Windows' own diagnostic data: Settings >
  Privacy & security > Diagnostics & feedback. Turning off *Send optional diagnostic data* stops
  every event that is not required diagnostic data. Required diagnostic data cannot be turned off
  on Home and Pro; on Enterprise, Education and Server editions an administrator's diagnostic
  data policy can turn it off too ("Diagnostic data off"). *Not verified:* which level (required or optional) each
  SDK event is sent at; that decides which of them the switch stops.
- **Inkwell, as the app.** There is no documented per-app switch that turns the SDK's events off,
  and Inkwell does not try to suppress Windows' telemetry machinery (that would be changing the
  user's system for them). What the app does: it uses none of the SDK features that add data
  collection (above), and it says so here and in its privacy text.
- **Not our concern here:** the .NET runtime compiled into Inkwell.exe sends nothing (.NET's
  telemetry is the SDK's command-line tools', on the build machine only, and the release workflow
  turns it off with `DOTNET_CLI_TELEMETRY_OPTOUT`).

## What to say to users

For the privacy text and the homepage (the release-prep step owns their wording): Inkwell sends
nothing itself; the Windows App SDK that draws its windows is part of Windows' diagnostic data,
controlled like the rest of it in Settings > Privacy & security > Diagnostics & feedback, and it
never sees what you dictate or record.

## To settle what is not verified

On a PC with *Send optional diagnostic data* on, start Inkwell with a TraceLogging session
listening to the SDK's providers (Windows Performance Recorder, or `tracelog`), use it for a few
minutes (dictate, open Settings and the Library), and read which events fired and at which
keyword. Then the same with optional diagnostic data off, and the Diagnostic Data Viewer
(Settings > Privacy & security > Diagnostics & feedback) for what actually left the PC.

## Sources

- [Microsoft's answer on the SDK's telemetry (WindowsAppSDK discussion #1941, January 2022)](https://github.com/microsoft/WindowsAppSDK/discussions/1941)
- [Configure Windows diagnostic data in your organization (Microsoft Learn)](https://learn.microsoft.com/en-us/windows/privacy/configure-windows-diagnostic-data-in-your-organization): the three settings, and "Diagnostic data off" on Enterprise, Education and Server editions only
- [WindowsAppSDK README, "Data collection"](https://github.com/microsoft/WindowsAppSDK/blob/main/README.md)
- [`dev/WindowsAppRuntime_Insights/WindowsAppRuntimeInsights.h`](https://github.com/microsoft/WindowsAppSDK/blob/main/dev/WindowsAppRuntime_Insights/WindowsAppRuntimeInsights.h), [`dev/Common/TelemetryHelper.h`](https://github.com/microsoft/WindowsAppSDK/blob/main/dev/Common/TelemetryHelper.h), [`dev/AppLifecycle/AppLifecycleTelemetry.h`](https://github.com/microsoft/WindowsAppSDK/blob/main/dev/AppLifecycle/AppLifecycleTelemetry.h)
- The Microsoft Software License Terms for the Windows App SDK, section 2(a) (the package's `license.txt`, shown in Settings > About)
