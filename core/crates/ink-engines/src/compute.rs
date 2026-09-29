//! Where a ggml engine computes: on a GPU when the build and the machine have one, else on the CPU
//! with one thread per physical core.
//!
//! The choice is made from the devices ggml reports at run time, not from the build: a Windows
//! build with Vulkan (`engine-llama-vulkan`) runs on the GPU where the machine has a Vulkan device
//! and on the CPU where it has none, and a build without a GPU backend always runs on the CPU. The
//! Mac's Metal device is a GPU like any other.
//!
//! - **GPU:** every layer offloaded, and llama.cpp's own thread defaults for the little work left
//!   on the CPU. That is the setup the speeds were measured with, on Metal and on Vulkan.
//! - **CPU:** nothing offloaded, and the thread count set explicitly to the machine's physical
//!   cores. llama.cpp's default is 4 threads whatever the machine, which leaves most of a
//!   desktop's cores idle while the model reads the audio (writing the text is bound by memory
//!   bandwidth more than by cores). Hyperthreads share a core's arithmetic units, so counting them
//!   only adds contention.
//!
//! [`choose`] is the rule, a pure function tested here on every OS; the llama.cpp adapter feeds it
//! ggml's device list ([`crate::llama::compute`]).

use std::fmt;
use std::num::NonZeroU32;

/// What kind of device ggml reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// The CPU.
    Cpu,
    /// A discrete GPU.
    Gpu,
    /// A GPU sharing the CPU's memory.
    IntegratedGpu,
    /// An accelerator that is not a GPU (for example a BLAS library): it does not take a model's
    /// layers.
    Accelerator,
    /// Anything else.
    Unknown,
}

/// A compute device as ggml reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The ggml backend it belongs to (for example `Vulkan`, `Metal`, `CPU`).
    pub backend: String,
    /// The device's own description (for example the GPU's product name).
    pub description: String,
    /// Its kind.
    pub kind: DeviceKind,
}

/// Where an engine computes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compute {
    /// On this GPU, every layer offloaded.
    Gpu {
        /// The ggml backend (for example `Vulkan` or `Metal`).
        backend: String,
        /// The device's description.
        description: String,
    },
    /// On the CPU alone, with this many threads.
    Cpu {
        /// Threads for generation and for reading the prompt: the machine's physical cores.
        threads: NonZeroU32,
    },
}

impl Compute {
    /// **Any thread.** Whether this is a GPU.
    pub fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu { .. })
    }

    /// **Any thread.** The thread count to set explicitly: `Some` on the CPU, `None` on a GPU
    /// (llama.cpp's defaults, as measured).
    pub fn cpu_threads(&self) -> Option<NonZeroU32> {
        match self {
            Self::Gpu { .. } => None,
            Self::Cpu { threads } => Some(*threads),
        }
    }
}

impl fmt::Display for Compute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gpu {
                backend,
                description,
            } => write!(f, "{backend} ({description})"),
            Self::Cpu { threads } => write!(f, "CPU ({threads} threads)"),
        }
    }
}

/// **Any thread.** The rule: the first discrete GPU, else the first integrated GPU, else the CPU
/// with `cpu_threads` threads. Devices are taken in ggml's order, which is the order llama.cpp
/// would use them in.
pub fn choose(devices: &[Device], cpu_threads: NonZeroU32) -> Compute {
    let first = |kind| devices.iter().find(|d| d.kind == kind);
    match first(DeviceKind::Gpu).or_else(|| first(DeviceKind::IntegratedGpu)) {
        Some(d) => Compute::Gpu {
            backend: d.backend.clone(),
            description: d.description.clone(),
        },
        None => Compute::Cpu {
            threads: cpu_threads,
        },
    }
}

/// **Any thread** (asks the OS; not for a realtime thread). The machine's physical cores (at
/// least 1): the thread count for CPU inference. Where the OS does not report cores, its logical
/// processor count (num_cpus's fallback).
pub fn physical_cores() -> NonZeroU32 {
    let cores = u32::try_from(num_cpus::get_physical()).unwrap_or(u32::MAX);
    NonZeroU32::new(cores).unwrap_or(NonZeroU32::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(backend: &str, description: &str, kind: DeviceKind) -> Device {
        Device {
            backend: backend.into(),
            description: description.into(),
            kind,
        }
    }

    fn threads(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[test]
    fn a_vulkan_gpu_is_chosen_over_the_cpu() {
        let devices = [
            device("Vulkan", "Synthetic GPU 9000", DeviceKind::Gpu),
            device("CPU", "Synthetic CPU", DeviceKind::Cpu),
        ];
        let chosen = choose(&devices, threads(12));
        assert_eq!(
            chosen,
            Compute::Gpu {
                backend: "Vulkan".into(),
                description: "Synthetic GPU 9000".into()
            }
        );
        assert!(chosen.is_gpu());
        // On a GPU the thread count is left to llama.cpp, as measured.
        assert_eq!(chosen.cpu_threads(), None);
    }

    #[test]
    fn with_no_gpu_the_cpu_gets_every_physical_core() {
        // What a Vulkan build reports on a machine with no Vulkan device, and every CPU-only build.
        let devices = [device("CPU", "Synthetic CPU", DeviceKind::Cpu)];
        let chosen = choose(&devices, threads(12));
        assert_eq!(
            chosen,
            Compute::Cpu {
                threads: threads(12)
            }
        );
        assert!(!chosen.is_gpu());
        assert_eq!(chosen.cpu_threads(), Some(threads(12)));
        assert_eq!(chosen.to_string(), "CPU (12 threads)");
        // No device list at all is the CPU too.
        assert_eq!(
            choose(&[], threads(1)),
            Compute::Cpu {
                threads: threads(1)
            }
        );
    }

    #[test]
    fn an_integrated_gpu_counts_but_a_discrete_one_comes_first() {
        // ggml's Vulkan backend lists only devices Vulkan calls discrete or integrated GPUs, so a
        // software rasteriser (a CPU-type Vulkan device) never reaches this list.
        let igpu = device("Vulkan", "Synthetic iGPU", DeviceKind::IntegratedGpu);
        let dgpu = device("Vulkan", "Synthetic dGPU", DeviceKind::Gpu);
        let cpu = device("CPU", "Synthetic CPU", DeviceKind::Cpu);
        // The Mac's Metal device may report either kind; both are the GPU.
        assert!(choose(&[cpu.clone(), igpu.clone()], threads(8)).is_gpu());
        let chosen = choose(&[igpu, cpu, dgpu], threads(8));
        assert_eq!(chosen.to_string(), "Vulkan (Synthetic dGPU)");
    }

    #[test]
    fn accelerators_and_unknown_devices_do_not_take_the_model() {
        let devices = [
            device("BLAS", "Synthetic BLAS", DeviceKind::Accelerator),
            device("Other", "Synthetic device", DeviceKind::Unknown),
            device("CPU", "Synthetic CPU", DeviceKind::Cpu),
        ];
        assert_eq!(
            choose(&devices, threads(6)),
            Compute::Cpu {
                threads: threads(6)
            }
        );
    }

    #[test]
    fn physical_cores_are_real_cores_not_hyperthreads() {
        let cores = physical_cores().get();
        let logical = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert!(cores >= 1);
        assert!(
            cores as usize <= logical,
            "{cores} physical cores but {logical} logical processors"
        );
    }
}
