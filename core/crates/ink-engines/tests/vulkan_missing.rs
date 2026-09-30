//! A Windows Vulkan build on a PC without the Vulkan loader (`vulkan-1.dll`, which GPU drivers
//! install): it starts, and llama.cpp chooses the CPU (`src/llama/no_vulkan.rs`).
//!
//! This test binary is linked as the core's DLL is, with the loader delay-loaded (ink-engines'
//! build script). Before anything touches ggml, the process's DLL search is narrowed to its own
//! directory, so the loader in System32 cannot be found: as far as the process can tell, the PC
//! has no Vulkan. One test per binary: the search and ggml's backend are process-wide.
//!
//! It needs no model and no GPU, so it runs wherever the Vulkan build is built (the release
//! workflow runs it). `tests/vulkan_present.rs` is the other half, on a PC with a
//! Vulkan GPU.
//!
//! ```text
//! cargo test -p ink-engines --features engine-llama-vulkan --test vulkan_missing
//! ```

#![cfg(all(windows, feature = "engine-llama-vulkan"))]

use std::ffi::{c_char, c_void};

/// `LOAD_LIBRARY_SEARCH_APPLICATION_DIR`: the executable's directory only.
const SEARCH_APPLICATION_DIR: u32 = 0x0000_0200;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetDefaultDllDirectories(flags: u32) -> i32;
    fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
    fn LoadLibraryExA(name: *const c_char, file: *mut c_void, flags: u32) -> *mut c_void;
}

#[test]
fn without_the_vulkan_loader_the_core_starts_and_llama_cpp_runs_on_the_cpu() {
    // SAFETY: plain Win32 calls with a flag constant and NUL-terminated names.
    unsafe {
        assert_ne!(
            SetDefaultDllDirectories(SEARCH_APPLICATION_DIR),
            0,
            "the DLL search was not narrowed"
        );
        assert!(
            GetModuleHandleA(c"vulkan-1.dll".as_ptr()).is_null(),
            "the Vulkan loader is loaded already: it is not delay-loaded"
        );
        // The case under test: the loader cannot be found with the search the delay-load helper uses.
        assert!(
            LoadLibraryExA(c"vulkan-1.dll".as_ptr(), std::ptr::null_mut(), 0).is_null(),
            "the Vulkan loader is found beside the test binary: the case cannot be made"
        );
    }

    // Starts llama.cpp's backend, which registers ggml's Vulkan backend. Without the hook, the
    // delay-load helper would raise here and end the process.
    let compute = ink_engines::llama::compute().expect("the llama.cpp backend starts");
    assert!(
        !compute.is_gpu(),
        "no Vulkan loader, but llama.cpp computes on {compute}"
    );
    assert!(compute.cpu_threads().is_some());
    // And it was the stand-in for the missing loader that ggml met, not some other failure.
    assert!(
        ink_engines::llama::vulkan_loader_missing(),
        "ggml never asked the stand-in: the hook did not answer"
    );

    // SAFETY: as above.
    unsafe {
        assert!(
            GetModuleHandleA(c"vulkan-1.dll".as_ptr()).is_null(),
            "a Vulkan loader was loaded"
        );
    }
}
