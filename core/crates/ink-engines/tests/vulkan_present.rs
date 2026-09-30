//! A Windows Vulkan build on a PC with a Vulkan GPU: with the Vulkan loader delay-loaded and the
//! hook for a missing one linked in (`src/llama/no_vulkan.rs`), llama.cpp still finds the GPU.
//! `#[ignore]`: it needs a Vulkan GPU and its driver, which CI does not have. No model.
//!
//! ```text
//! cargo test -p ink-engines --features engine-llama-vulkan --test vulkan_present -- --ignored
//! ```

#![cfg(all(windows, feature = "engine-llama-vulkan"))]

#[test]
#[ignore = "needs a Vulkan GPU"]
fn with_a_vulkan_gpu_llama_cpp_runs_on_it() {
    let compute = ink_engines::llama::compute().expect("the llama.cpp backend starts");
    assert!(
        compute.is_gpu(),
        "a Vulkan GPU is here, but llama.cpp computes on {compute}"
    );
    assert!(compute.to_string().starts_with("Vulkan"), "{compute}");
    assert!(!ink_engines::llama::vulkan_loader_missing());
}
