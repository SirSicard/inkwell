use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStyleRule {
    pub process_name: String,  // e.g. "outlook.exe", "slack.exe"
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
        let process = get_foreground_process_name()?;
        let process_lower = process.to_lowercase();
        self.rules.iter()
            .find(|r| process_lower.contains(&r.process_name.to_lowercase()))
            .map(|r| r.style.clone())
    }

    fn default_rules() -> Self {
        Self {
            enabled: false,
            rules: vec![
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
            ],
        }
    }
}

/// Get the process name of the foreground window (Windows only).
#[cfg(target_os = "windows")]
fn get_foreground_process_name() -> Option<String> {
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

#[cfg(not(target_os = "windows"))]
fn get_foreground_process_name() -> Option<String> {
    // TODO: macOS via NSWorkspace, Linux via xdotool/wmctrl
    None
}
