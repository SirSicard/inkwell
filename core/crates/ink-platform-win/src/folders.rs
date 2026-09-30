//! Known folders: where other apps keep their data.
#![cfg(windows)]

use std::path::PathBuf;

use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

/// The user's roaming application data folder (`FOLDERID_RoamingAppData`, what `%APPDATA%`
/// names), or `None` when Windows cannot say. Tauri apps, Inkwell 0.2 among them, keep their data
/// in a folder there named after the app's identifier.
///
/// **Any thread.** No COM apartment is needed; nothing is opened.
pub fn roaming_app_data() -> Option<PathBuf> {
    // SAFETY: a known folder id, default flags and no token (the calling user). On success the
    // string is the caller's, allocated with CoTaskMemAlloc.
    let path =
        unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None) }.ok()?;
    // SAFETY: as above: a NUL-terminated wide string this function owns, freed there.
    unsafe { crate::com::take_co_string(path) }.map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the path is compared: nothing in the folder is opened.
    #[test]
    fn it_is_the_folder_appdata_names() {
        let folder = roaming_app_data().expect("a signed-in user has one");
        assert!(folder.is_absolute(), "{folder:?}");
        if let Some(appdata) = std::env::var_os("APPDATA") {
            assert_eq!(folder, PathBuf::from(appdata));
        }
    }
}
