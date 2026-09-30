//! A Vulkan build on a PC without Vulkan (Windows, `engine-llama-vulkan`): the core still starts,
//! and llama.cpp runs on the CPU.
//!
//! ggml's Vulkan backend is linked in statically and imports a few functions from the Vulkan loader,
//! `vulkan-1.dll`, which GPU drivers install. Windows refuses to load a DLL whose load-time imports
//! are missing, so a core that imported them directly would not start at all on a PC without a
//! Vulkan driver. The core's DLL therefore delay-loads `vulkan-1.dll`
//! (ink-ffi's build script passes `/DELAYLOAD`): nothing is loaded until ggml registers its Vulkan
//! backend, the first time the llama.cpp backend starts ([`super::compute`]).
//!
//! If the loader is missing at that moment, MSVC's delay-load helper would raise a structured
//! exception, which ggml's C++ cannot catch. This module installs the helper's notify hook
//! (`__pfnDliNotifyHook2`, from `delayimp.h`), which runs before the helper loads anything:
//!
//! - for anything but `vkGetInstanceProcAddr` from `vulkan-1.dll`, and whenever `vulkan-1.dll`
//!   loads (tried with the helper's own search), it steps aside and the helper works as usual;
//! - otherwise it answers with a stand-in `vkGetInstanceProcAddr` whose only function,
//!   `vkEnumerateInstanceVersion`, fails with `VK_ERROR_INITIALIZATION_FAILED`. ggml's Vulkan
//!   registration catches the error Vulkan-Hpp throws for it and registers no Vulkan device, so
//!   [`super::compute`] chooses the CPU and logs that the PC has no Vulkan loader
//!   ([`super::vulkan_loader_missing`]).
//!
//! (Without the hook the build we ship happens to survive too: cmake-rs compiles llama.cpp without
//! `/EHsc`, and then ggml's `catch (...)` also takes the helper's structured exception. That rests
//! on a compiler flag nobody chose; the hook does not.)
//!
//! `vkGetInstanceProcAddr` is the first of them ggml calls, and without a loader nothing gets past
//! it. The others (`vkGetDeviceProcAddr`, `vkGetPhysicalDeviceFeatures2`, `vkCmdCopyBuffer` in
//! llama-cpp-sys-2 0.1.157) take a device or a physical device, which exist only once an instance
//! was created through a real loader; a missing loader would still raise on one called before
//! that. `windows/scripts/build-core.ps1` fails a release whose delay-load table names a function
//! not checked this way, and `tests/vulkan_missing.rs` runs the case in a process that cannot find
//! the loader.

#![warn(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

/// `dliStartProcessing` (delayimp.h): the first call for each import the helper resolves. A
/// non-null answer is used as the function, and the helper does nothing else.
const DLI_START_PROCESSING: u32 = 0;

/// `VK_ERROR_INITIALIZATION_FAILED` (vulkan_core.h).
const VK_ERROR_INITIALIZATION_FAILED: i32 = -3;

/// `DelayLoadProc` (delayimp.h): the import, by name or by ordinal.
#[repr(C)]
struct DelayLoadProc {
    /// Non-zero when the import is by name (`name` is then a C string).
    import_by_name: i32,
    /// The name, or, by ordinal, the ordinal in the pointer's low bits (a union in C).
    name: *const c_char,
}

/// `DelayLoadInfo` (delayimp.h), as the helper passes it to its hooks.
#[repr(C)]
struct DelayLoadInfo {
    size: u32,
    descriptor: *const c_void,
    import_address: *mut *const c_void,
    dll: *const c_char,
    proc: DelayLoadProc,
    module: *mut c_void,
    function: *const c_void,
    last_error: u32,
}

/// `PfnDliHook` (delayimp.h).
type DliHook = unsafe extern "system" fn(u32, *const DelayLoadInfo) -> *const c_void;

// The helper (delayimp.lib) reads this pointer. delayimp.lib defines a null one in a member of its
// own, which the linker takes only while no other definition is linked: `keep` references this one
// so that it is linked first.
#[unsafe(no_mangle)]
#[used]
#[allow(non_upper_case_globals)]
static __pfnDliNotifyHook2: Option<DliHook> = Some(notify);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExA(name: *const c_char, file: *mut c_void, flags: u32) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}

/// Set when ggml asked the stand-in whether Vulkan runs: this process found no loader.
static STOOD_IN: AtomicBool = AtomicBool::new(false);

/// **Any thread.** Links the hook into every binary that starts the llama.cpp backend (see the
/// comment on `__pfnDliNotifyHook2`); does nothing else.
pub(super) fn keep() {
    std::hint::black_box(&raw const __pfnDliNotifyHook2);
}

/// **Any thread.** Whether ggml met the stand-in when it started: the PC has no Vulkan loader.
pub(super) fn loader_missing() -> bool {
    STOOD_IN.load(Ordering::Relaxed)
}

/// The notify hook. Runs on the thread that first calls a delay-loaded import (a worker starting
/// the llama.cpp backend), inside the helper: it allocates nothing and logs nothing.
unsafe extern "system" fn notify(event: u32, info: *const DelayLoadInfo) -> *const c_void {
    if event != DLI_START_PROCESSING || info.is_null() {
        return ptr::null();
    }
    // SAFETY: the helper passes a valid DelayLoadInfo for the duration of the call.
    let info = unsafe { &*info };
    if info.dll.is_null() || info.proc.import_by_name == 0 || info.proc.name.is_null() {
        return ptr::null();
    }
    // SAFETY: both are NUL-terminated strings in the DLL's delay-import tables.
    let (dll, name) = unsafe { (CStr::from_ptr(info.dll), CStr::from_ptr(info.proc.name)) };
    if !dll.to_bytes().eq_ignore_ascii_case(b"vulkan-1.dll")
        || name.to_bytes() != b"vkGetInstanceProcAddr"
        || loads(info.dll)
    {
        return ptr::null();
    }
    no_vulkan_get_instance_proc_addr as *const c_void
}

/// Whether `dll` loads with the search the helper itself uses (flags 0). The reference is dropped
/// again: the helper takes its own when it goes on.
fn loads(dll: *const c_char) -> bool {
    // SAFETY: `dll` is a NUL-terminated string; no file handle, default flags.
    let module = unsafe { LoadLibraryExA(dll, ptr::null_mut(), 0) };
    if module.is_null() {
        return false;
    }
    // SAFETY: `module` was just loaded above, and this releases that one reference.
    unsafe { FreeLibrary(module) };
    true
}

/// The stand-in for `vkGetInstanceProcAddr` (`PFN_vkGetInstanceProcAddr`): no instance-level
/// function exists but one that says Vulkan did not start.
unsafe extern "system" fn no_vulkan_get_instance_proc_addr(
    _instance: *mut c_void,
    name: *const c_char,
) -> *const c_void {
    if name.is_null() {
        return ptr::null();
    }
    // SAFETY: Vulkan-Hpp passes NUL-terminated function names.
    if unsafe { CStr::from_ptr(name) }.to_bytes() == b"vkEnumerateInstanceVersion" {
        return no_vulkan_instance_version as *const c_void;
    }
    ptr::null()
}

/// The stand-in for `vkEnumerateInstanceVersion` (`PFN_vkEnumerateInstanceVersion`).
unsafe extern "system" fn no_vulkan_instance_version(_version: *mut u32) -> i32 {
    STOOD_IN.store(true, Ordering::Relaxed);
    VK_ERROR_INITIALIZATION_FAILED
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(dll: &CStr, name: &CStr) -> DelayLoadInfo {
        DelayLoadInfo {
            size: size_of::<DelayLoadInfo>() as u32,
            descriptor: ptr::null(),
            import_address: ptr::null_mut(),
            dll: dll.as_ptr(),
            proc: DelayLoadProc {
                import_by_name: 1,
                name: name.as_ptr(),
            },
            module: ptr::null_mut(),
            function: ptr::null(),
            last_error: 0,
        }
    }

    #[test]
    fn the_structures_match_delayimp_h_on_x64() {
        // DelayLoadProc is a BOOL and a pointer-sized union; DelayLoadInfo as the helper fills it.
        assert_eq!(size_of::<DelayLoadProc>(), 16);
        assert_eq!(size_of::<DelayLoadInfo>(), 72);
        assert_eq!(std::mem::offset_of!(DelayLoadInfo, proc), 32);
        assert_eq!(std::mem::offset_of!(DelayLoadInfo, last_error), 64);
    }

    #[test]
    fn the_hook_steps_aside_for_other_imports_and_other_events() {
        let other_dll = info(c"other.dll", c"vkGetInstanceProcAddr");
        let other_name = info(c"vulkan-1.dll", c"vkCreateInstance");
        let wanted = info(c"no-such-loader-for-inkwell.dll", c"vkGetInstanceProcAddr");
        // SAFETY: each info is valid for the call.
        unsafe {
            assert!(notify(DLI_START_PROCESSING, &other_dll).is_null());
            assert!(notify(DLI_START_PROCESSING, &other_name).is_null());
            assert!(notify(DLI_START_PROCESSING, ptr::null()).is_null());
            // dliNotePreLoadLibrary (1) and the others: never answered.
            assert!(notify(1, &wanted).is_null());
        }
    }

    #[test]
    fn the_stand_in_says_vulkan_did_not_start() {
        // SAFETY: C string names; the stand-ins take any pointers.
        unsafe {
            let version = no_vulkan_get_instance_proc_addr(
                ptr::null_mut(),
                c"vkEnumerateInstanceVersion".as_ptr(),
            );
            assert!(!version.is_null());
            let version = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(*mut u32) -> i32,
            >(version);
            let mut out = 0u32;
            assert_eq!(version(&mut out), VK_ERROR_INITIALIZATION_FAILED);
            for name in [
                c"vkCreateInstance",
                c"vkEnumerateInstanceExtensionProperties",
                c"vkEnumerateInstanceLayerProperties",
            ] {
                assert!(no_vulkan_get_instance_proc_addr(ptr::null_mut(), name.as_ptr()).is_null());
            }
            assert!(no_vulkan_get_instance_proc_addr(ptr::null_mut(), ptr::null()).is_null());
        }
    }

    #[test]
    fn a_missing_loader_is_answered_with_the_stand_in() {
        // A DLL name no machine has, standing in for vulkan-1.dll: the hook matches the name only
        // for vulkan-1.dll, so this checks `loads` directly and the answer for the real name
        // through the test in tests/vulkan_missing.rs.
        assert!(!loads(c"no-such-loader-for-inkwell.dll".as_ptr()));
        assert!(loads(c"kernel32.dll".as_ptr()));
    }
}
