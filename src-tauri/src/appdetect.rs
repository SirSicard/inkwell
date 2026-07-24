use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStyleRule {
    /// Substring matched against the foreground app's identity: the executable
    /// name on Windows ("outlook.exe") or the bundle identifier on macOS
    /// ("com.microsoft.Outlook"). Field name kept for settings-file compat.
    pub process_name: String,
    pub style: String,         // "formal", "casual", "relaxed"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppStyleRules {
    pub enabled: bool,
    pub rules: Vec<AppStyleRule>,
}

impl AppStyleRules {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default_rules(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(path, json).map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    /// Returns the style override for the currently focused app, or None if no match.
    pub fn get_override(&self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let app_id = get_foreground_app_id()?;
        let app_id_lower = app_id.to_lowercase();
        self.rules.iter()
            .find(|r| app_id_lower.contains(&r.process_name.to_lowercase()))
            .map(|r| r.style.clone())
    }

    /// Defaults cover both identity shapes. A Windows box never sees a bundle
    /// id and a Mac never sees an .exe, so one list serves both.
    fn default_rules() -> Self {
        Self {
            enabled: false,
            rules: vec![
                // Windows: executable names
                AppStyleRule { process_name: "outlook.exe".into(), style: "formal".into() },
                AppStyleRule { process_name: "thunderbird.exe".into(), style: "formal".into() },
                AppStyleRule { process_name: "slack.exe".into(), style: "casual".into() },
                AppStyleRule { process_name: "discord.exe".into(), style: "casual".into() },
                AppStyleRule { process_name: "teams.exe".into(), style: "casual".into() },
                AppStyleRule { process_name: "ms-teams.exe".into(), style: "casual".into() },
                AppStyleRule { process_name: "whatsapp.exe".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "telegram.exe".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "signal.exe".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "code.exe".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "notepad.exe".into(), style: "relaxed".into() },
                // macOS: bundle identifiers
                AppStyleRule { process_name: "com.apple.mail".into(), style: "formal".into() },
                AppStyleRule { process_name: "com.microsoft.Outlook".into(), style: "formal".into() },
                AppStyleRule { process_name: "com.readdle.smartemail-Mac".into(), style: "formal".into() },
                AppStyleRule { process_name: "org.mozilla.thunderbird".into(), style: "formal".into() },
                AppStyleRule { process_name: "com.tinyspeck.slackmacgap".into(), style: "casual".into() },
                AppStyleRule { process_name: "com.hnc.Discord".into(), style: "casual".into() },
                AppStyleRule { process_name: "com.microsoft.teams".into(), style: "casual".into() },
                AppStyleRule { process_name: "us.zoom.xos".into(), style: "casual".into() },
                AppStyleRule { process_name: "net.whatsapp.WhatsApp".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "ru.keepcoder.Telegram".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "org.whispersystems.signal-desktop".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "com.apple.MobileSMS".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "com.microsoft.VSCode".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "com.apple.dt.Xcode".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "com.apple.TextEdit".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "com.apple.Terminal".into(), style: "relaxed".into() },
                AppStyleRule { process_name: "com.googlecode.iterm2".into(), style: "relaxed".into() },
            ],
        }
    }
}

/// Identity of the frontmost app: executable file name on Windows.
#[cfg(target_os = "windows")]
fn get_foreground_app_id() -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }

        let handle = OpenProcess(0x0400 | 0x0010, 0, pid); // PROCESS_QUERY_INFORMATION | PROCESS_VM_READ
        if handle.is_null() {
            return None;
        }

        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);

        if ok == 0 {
            return None;
        }

        let path = OsString::from_wide(&buf[..size as usize]);
        let path_str = path.to_string_lossy().to_string();

        // Extract just the filename
        path_str.rsplit('\\').next().map(|s| s.to_string())
    }
}

#[cfg(target_os = "windows")]
extern "system" {
    fn GetForegroundWindow() -> *mut std::ffi::c_void;
    fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, pid: *mut u32) -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn QueryFullProcessImageNameW(handle: *mut std::ffi::c_void, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

/// Identity of the frontmost app: bundle identifier on macOS
/// (e.g. "com.apple.Safari"), via NSWorkspace.frontmostApplication.
///
/// Hand-rolled objc dispatch because `cocoa` re-exports the runtime types but
/// not the `msg_send!` macro, and nothing in the tree exposes NSWorkspace.
// The cocoa crate is deprecated in favour of objc2-app-kit. That migration is
// deferred to its own change; this code is correct against cocoa as pinned.
#[allow(deprecated)]
#[cfg(target_os = "macos")]
fn get_foreground_app_id() -> Option<String> {
    use cocoa::base::{id, nil, selector, SEL};
    use cocoa::foundation::NSString;
    use std::ffi::CStr;
    use std::os::raw::c_char;

    // objc_msgSend is variadic in C. This fixed (receiver, selector) signature
    // is only sound because every send below passes zero further arguments.
    extern "C" {
        fn objc_getClass(name: *const c_char) -> id;
        fn objc_msgSend(receiver: id, sel: SEL) -> id;
    }

    // NSWorkspace lives in AppKit; force the framework link from this module
    // rather than relying on another module happening to pull it in.
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    unsafe {
        let class = objc_getClass(c"NSWorkspace".as_ptr());
        if class == nil {
            return None;
        }
        let workspace = objc_msgSend(class, selector("sharedWorkspace"));
        if workspace == nil {
            return None;
        }
        // nil when the frontmost app is not a regular app (e.g. the login window).
        let front = objc_msgSend(workspace, selector("frontmostApplication"));
        if front == nil {
            return None;
        }
        // nil for processes without a bundle, such as a bare executable.
        let bundle_id = objc_msgSend(front, selector("bundleIdentifier"));
        if bundle_id == nil {
            return None;
        }
        let utf8 = bundle_id.UTF8String();
        if utf8.is_null() {
            return None;
        }
        Some(CStr::from_ptr(utf8).to_string_lossy().into_owned())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn get_foreground_app_id() -> Option<String> {
    // Linux is best-effort: X11 needs xdotool/wmctrl and Wayland forbids it
    // outright, so per-app style overrides stay off here.
    None
}
