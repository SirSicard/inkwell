//! The machine's free disk space (`GetDiskFreeSpaceExW`).
#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use ink_core::{PlatformError, SystemInfo};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::core::PCWSTR;

/// [`SystemInfo`] on Windows. Stateless: each call asks the OS.
///
/// **Any thread.** No COM apartment is needed; nothing is opened.
#[derive(Clone, Copy, Debug, Default)]
pub struct WinSystemInfo;

impl SystemInfo for WinSystemInfo {
    fn free_disk_bytes(&self, path: &Path) -> Result<u64, PlatformError> {
        // Said as such, whatever GetDiskFreeSpaceExW would answer for it: the contract asks for
        // an existing path. A path that cannot be checked is said as that, not as missing.
        match path.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                return Err(PlatformError::Failed(
                    "free space: the path does not exist".into(),
                ));
            }
            Err(e) => {
                return Err(PlatformError::Failed(format!(
                    "free space: the path cannot be checked: {e}"
                )));
            }
        }
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err(PlatformError::Failed(
                "free space: the path holds a NUL character".into(),
            ));
        }
        let mut available: u64 = 0;
        // SAFETY: `wide` is a NUL-terminated wide string that outlives the call; the one out
        // pointer is to a live u64, and the two totals are not asked for.
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR::from_raw(wide.as_ptr()),
                Some(&raw mut available),
                None,
                None,
            )
        }
        .map_err(|e| PlatformError::Failed(format!("free space: GetDiskFreeSpaceExW: {e}")))?;
        // The bytes free to this caller (its quota), not the volume's total free.
        Ok(available)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_holding_the_temp_directory_has_free_space() {
        let free = WinSystemInfo
            .free_disk_bytes(&std::env::temp_dir())
            .unwrap();
        assert!(free > 0);
    }

    #[test]
    fn a_missing_path_is_an_error_not_zero() {
        let e = WinSystemInfo
            .free_disk_bytes(Path::new(r"C:\no\such\inkwell\path"))
            .unwrap_err();
        assert!(matches!(e, PlatformError::Failed(_)), "{e}");
    }
}
