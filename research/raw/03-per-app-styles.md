# Per-App Style Overrides - Research

## Windows API Chain
- `GetForegroundWindow()` → HWND
- `GetWindowThreadProcessId(hwnd)` → PID  
- `OpenProcess(PID)` → HANDLE
- `QueryFullProcessImageNameW(handle)` → full exe path
- `GetWindowTextW(hwnd)` → window title

All ~1ms. Call on-demand when transcription completes, no background polling needed.

## Rust Crate
`windows` crate (Microsoft official), features: `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading`, `Win32_Foundation`

## Data Model
- **TextStyle**: id, label, formatting rules (capitalization, punctuation, filler words, paragraphs)
- **AppRule**: match by process name, process path, window title, or regex
- **Resolution**: first matching rule wins, fallback to default style
- Config stored as JSON in app data dir

## How Others Do It
- **Espanso**: `filter_exec` (process name substring), `filter_title` (window title). C++ native calls, same Win32 chain.
- **TextExpander**: expansion groups restricted by executable name
- **PhraseExpress**: folder-level program assignment

All use process name as primary matcher. Window title is secondary. No regex in primary matching.

## UX
- "Detect current app" button: user focuses target app, clicks detect, auto-fills process name
- Show active style in tray/overlay: "Style: Casual (Slack)"
- Ship presets: casual (slack, discord, telegram), formal (outlook, thunderbird), technical (vscode)

## Cross-Platform
- macOS: `NSWorkspace.shared.frontmostApplication` (no permissions for app name, AX API for window title)
- Linux: X11 `_NET_ACTIVE_WINDOW`, Wayland needs compositor-specific protocols
