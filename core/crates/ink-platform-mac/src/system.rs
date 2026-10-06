//! The machine's free disk space (`statfs`) and physical memory (`sysctl hw.memsize`).

#![cfg(target_os = "macos")]

use std::ffi::CString;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use ink_core::{PlatformError, SystemInfo};

/// [`SystemInfo`] on macOS. Stateless: each call asks the kernel.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacSystemInfo;

impl SystemInfo for MacSystemInfo {
    fn free_disk_bytes(&self, path: &Path) -> Result<u64, PlatformError> {
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| PlatformError::Failed("free space: the path holds a NUL byte".into()))?;
        let mut out = MaybeUninit::<libc::statfs>::zeroed();
        // SAFETY: `c_path` is a NUL-terminated string that outlives the call, and `out` points to
        // writable memory of the size `statfs` fills.
        let status = unsafe { libc::statfs(c_path.as_ptr(), out.as_mut_ptr()) };
        if status != 0 {
            return Err(PlatformError::Failed(format!(
                "free space: statfs failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: `statfs` returned 0, so it filled `out`.
        let fs = unsafe { out.assume_init() };
        // Blocks available to an unprivileged user, not `f_bfree` (which counts the reserve).
        Ok(fs.f_bavail.saturating_mul(u64::from(fs.f_bsize)))
    }

    fn total_memory_bytes(&self) -> Result<u64, PlatformError> {
        let mut bytes: u64 = 0;
        let mut len = size_of::<u64>();
        // SAFETY: the name is a NUL-terminated literal; `bytes` and `len` are valid for writes,
        // and `len` holds `bytes`' size, which is what `hw.memsize` (a 64-bit integer) fills.
        let status = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut bytes).cast(),
                &raw mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || len != size_of::<u64>() {
            return Err(PlatformError::Failed(format!(
                "memory: sysctl hw.memsize failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_holding_the_temp_directory_has_free_space() {
        let free = MacSystemInfo
            .free_disk_bytes(&std::env::temp_dir())
            .unwrap();
        assert!(free > 0);
    }

    #[test]
    fn a_missing_path_is_an_error_not_zero() {
        let e = MacSystemInfo
            .free_disk_bytes(Path::new("/no/such/inkwell/path"))
            .unwrap_err();
        assert!(matches!(e, PlatformError::Failed(_)), "{e}");
    }

    #[test]
    fn the_mac_has_at_least_a_gigabyte_of_memory() {
        assert!(MacSystemInfo.total_memory_bytes().unwrap() >= 1 << 30);
    }
}
