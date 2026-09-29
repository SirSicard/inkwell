//! The ink shader, written once in WGSL (`shaders/ink.wgsl`) and translated with naga into what
//! each shell compiles: the Metal Shading Language for the Mac, HLSL (shader model 5.0, for
//! Direct3D 11) for Windows.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

/// `shaders/ink.wgsl`, as this crate was built with it.
pub const INK_WGSL: &str = include_str!("../../../../shaders/ink.wgsl");

/// Where the generated MSL lives, from the repository root. The Mac app bundles it as a resource
/// and compiles it at run time, so building the app needs no Metal toolchain.
pub const MSL_OUT: &str = "mac/Sources/InkRenderer/Resources/ink.msl";

/// Where the generated HLSL lives, from the repository root. The Windows shell embeds it and
/// compiles it at run time with the system's shader compiler (D3DCompile, shader model 5.0), so
/// building the app needs no shader toolchain either.
pub const HLSL_OUT: &str = "windows/Inkwell.Ink/Shaders/ink.hlsl";

/// Why a shader did not translate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderError {
    /// The WGSL does not parse. The message names the line.
    Parse(String),
    /// It parses but is not a valid module.
    Invalid(String),
    /// A binding the shells rely on is missing or has another type.
    Layout(String),
    /// The MSL back end refused it.
    Msl(String),
    /// The HLSL back end refused it, or its output was not the shape the Direct3D 11 fix-up
    /// expects (see [`hlsl`]).
    Hlsl(String),
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "the WGSL does not parse: {m}"),
            Self::Invalid(m) => write!(f, "the WGSL is not a valid module: {m}"),
            Self::Layout(m) => write!(
                f,
                "the shader's bindings are not the ones the shells use: {m}"
            ),
            Self::Msl(m) => write!(f, "the MSL back end refused the shader: {m}"),
            Self::Hlsl(m) => write!(f, "the HLSL back end refused the shader: {m}"),
        }
    }
}

impl std::error::Error for ShaderError {}

/// The MSL for `wgsl`, with every resource the fragment stage uses on the slot the Mac renderer
/// binds it to (see [`binding_map`]).
pub fn msl(wgsl: &str) -> Result<String, ShaderError> {
    let (module, info) = parse(wgsl)?;
    let map = binding_map();
    check_bindings(&module, &info, |entry, binding| {
        map.get(entry)
            .is_some_and(|resources| resources.resources.contains_key(binding))
    })?;
    let options = naga::back::msl::Options {
        // MSL 2.4 is macOS 12's; the app's floor is macOS 26. The app compiles the source with
        // the system's own compiler at run time, so this only sets what naga may emit.
        lang_version: (2, 4),
        per_entry_point_map: map,
        // A resource without a slot is an error here (check_bindings names it), never a
        // placeholder slot that Metal fills in on its own.
        fake_missing_bindings: false,
        // Everything else as naga's CLI sets it, which is what the ported shader was checked
        // against pixel for pixel: unchecked indexing (the only index is a loop over the six
        // droplets), and loops marked as bounded.
        ..naga::back::msl::Options::default()
    };
    let (source, _) = naga::back::msl::write_string(
        &module,
        &info,
        &options,
        &naga::back::msl::PipelineOptions::default(),
    )
    .map_err(|e| ShaderError::Msl(e.to_string()))?;
    Ok(source)
}

/// The HLSL for `wgsl`: shader model 5.0, which Direct3D 11 runs (DXC's shader model 6 output is
/// for Direct3D 12 only). Each resource sits on register 0 of its kind: the uniform block at
/// `b0`, the wordmark at `t0`, its sampler at `s0`, all in space 0 (see [`hlsl_binding_map`]).
///
/// naga writes every sampler through a sampler heap indexed by a buffer (`nagaSamplerHeap`), a
/// Direct3D 12 binding model that shader model 5.0 cannot express. The one sampler is rewritten
/// as a plain `SamplerState` on `s0`; the rewrite checks each line it replaces, so a naga whose
/// output changes shape fails here instead of compiling into something else.
pub fn hlsl(wgsl: &str) -> Result<String, ShaderError> {
    let (module, info) = parse(wgsl)?;
    let map = hlsl_binding_map();
    check_bindings(&module, &info, |_, binding| map.contains_key(binding))?;
    let mut sampler_buffers = naga::back::hlsl::SamplerIndexBufferBindingMap::new();
    // Only so naga can write the heap's index buffer, which the rewrite then removes.
    sampler_buffers.insert(
        naga::back::hlsl::SamplerIndexBufferKey { group: 0 },
        target(1),
    );
    let options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_0,
        binding_map: map,
        // A resource without a register is an error (check_bindings names it), never a guess.
        fake_missing_bindings: false,
        sampler_buffer_binding_map: sampler_buffers,
        ..naga::back::hlsl::Options::default()
    };
    let mut out = String::new();
    naga::back::hlsl::Writer::new(
        &mut out,
        &options,
        &naga::back::hlsl::PipelineOptions::default(),
    )
    .write(&module, &info, None)
    .map_err(|e| ShaderError::Hlsl(e.to_string()))?;
    plain_sampler(&out)
}

/// The registers of bind group 0 for HLSL: the uniform block on `b0`, the wordmark on `t0`, and
/// its sampler at index 0 of naga's sampler index buffer, which [`plain_sampler`] turns into `s0`.
pub fn hlsl_binding_map() -> naga::back::hlsl::BindingMap {
    let at = |binding| naga::ResourceBinding { group: 0, binding };
    let mut map = naga::back::hlsl::BindingMap::new();
    map.insert(at(0), target(0));
    map.insert(at(1), target(0));
    map.insert(at(2), target(0));
    map
}

fn target(register: u32) -> naga::back::hlsl::BindTarget {
    naga::back::hlsl::BindTarget {
        register,
        ..naga::back::hlsl::BindTarget::default()
    }
}

/// Replaces naga's sampler heap with one `SamplerState markSamp : register(s0);`.
fn plain_sampler(hlsl: &str) -> Result<String, ShaderError> {
    const HEAP: &str = "SamplerState nagaSamplerHeap[2048]: register(s0, space0);";
    const COMPARISON_HEAP: &str =
        "SamplerComparisonState nagaComparisonSamplerHeap[2048]: register(s0, space1);";
    const INDEX_BUFFER: &str =
        "StructuredBuffer<uint> nagaGroup0SamplerIndexArray : register(t1, space0);";
    const SAMPLER: &str =
        "static const SamplerState markSamp = nagaSamplerHeap[nagaGroup0SamplerIndexArray[0]];";
    const PLAIN: &str = "SamplerState markSamp : register(s0);";
    let mut out = String::with_capacity(hlsl.len());
    let mut seen = [0usize; 4];
    for line in hlsl.lines() {
        match line.trim_end() {
            HEAP => seen[0] += 1,
            COMPARISON_HEAP => seen[1] += 1,
            INDEX_BUFFER => seen[2] += 1,
            SAMPLER => {
                seen[3] += 1;
                out.push_str(PLAIN);
                out.push('\n');
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    if seen != [1, 1, 1, 1] || out.contains("nagaSamplerHeap") || out.contains("nagaGroup") {
        return Err(ShaderError::Hlsl(format!(
            "naga's sampler heap is not the expected four lines (found {seen:?}), so the \
             Direct3D 11 sampler rewrite cannot apply"
        )));
    }
    Ok(out)
}

/// The whole generated HLSL file: a header naming its source, then [`hlsl`].
pub fn hlsl_file(wgsl: &str) -> Result<String, ShaderError> {
    let body = hlsl(wgsl)?;
    let mut out = String::from(
        "// Generated from shaders/ink.wgsl by core/crates/ink-shader (naga). Do not edit: change the\n\
         // WGSL, then run `cargo run -p ink-shader --bin ink-shader` in core/.\n\
         //\n\
         // Shader model 5.0 (Direct3D 11, compiled at run time with D3DCompile). The uniform block U\n\
         // at b0, the wordmark at t0, its sampler at s0.\n",
    );
    out.push_str(&body);
    Ok(out)
}

/// The uniform block's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UniformLayout {
    /// Its size in bytes.
    pub size: u32,
    /// Where `drops` starts.
    pub drops_offset: u32,
}

/// The uniform block's layout in `wgsl`: the struct behind `var<uniform> u`, as the WGSL rules lay
/// it out. The shells fill it by these offsets.
pub fn uniform_layout(wgsl: &str) -> Result<UniformLayout, ShaderError> {
    let (module, _) = parse(wgsl)?;
    let (_, global) = module
        .global_variables
        .iter()
        .find(|(_, g)| g.name.as_deref() == Some("u") && g.space == naga::AddressSpace::Uniform)
        .ok_or_else(|| ShaderError::Layout("no `var<uniform> u`".into()))?;
    let naga::TypeInner::Struct { members, span } = &module.types[global.ty].inner else {
        return Err(ShaderError::Layout("`u` is not a struct".into()));
    };
    let drops = members
        .iter()
        .find(|m| m.name.as_deref() == Some("drops"))
        .ok_or_else(|| ShaderError::Layout("`u` has no `drops`".into()))?;
    Ok(UniformLayout {
        size: *span,
        drops_offset: drops.offset,
    })
}

/// The whole generated file: a header naming its source, then [`msl`].
pub fn msl_file(wgsl: &str) -> Result<String, ShaderError> {
    let body = msl(wgsl)?;
    let mut out = String::from(
        "// Generated from shaders/ink.wgsl by core/crates/ink-shader (naga). Do not edit: change the\n\
         // WGSL, then run `cargo run -p ink-shader --bin ink-shader` in core/.\n\
         //\n\
         // Fragment stage: the uniform block U at buffer 0, the wordmark at texture 0, its sampler\n\
         // at sampler 0.\n",
    );
    out.push_str(&body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// The slots of bind group 0 (the uniform block, the wordmark texture, its sampler), each on
/// slot 0 of its kind in the fragment stage. The vertex stage binds nothing.
pub fn binding_map() -> naga::back::msl::EntryPointResourceMap {
    use naga::back::msl::{BindSamplerTarget, BindTarget, EntryPointResources};
    let at = |binding| naga::ResourceBinding { group: 0, binding };
    let mut fragment = EntryPointResources::default();
    fragment.resources.insert(
        at(0),
        BindTarget {
            buffer: Some(0),
            ..BindTarget::default()
        },
    );
    fragment.resources.insert(
        at(1),
        BindTarget {
            texture: Some(0),
            ..BindTarget::default()
        },
    );
    fragment.resources.insert(
        at(2),
        BindTarget {
            sampler: Some(BindSamplerTarget::Resource(0)),
            ..BindTarget::default()
        },
    );
    let mut map = naga::back::msl::EntryPointResourceMap::new();
    map.insert("vs_main".into(), EntryPointResources::default());
    map.insert("fs_main".into(), fragment);
    map
}

fn parse(wgsl: &str) -> Result<(naga::Module, naga::valid::ModuleInfo), ShaderError> {
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|e| ShaderError::Parse(e.emit_to_string(wgsl)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| ShaderError::Invalid(e.emit_to_string(wgsl)))?;
    Ok((module, info))
}

/// Every resource an entry point uses has a slot: `bound(entry point, binding)`.
fn check_bindings(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    bound: impl Fn(&str, &naga::ResourceBinding) -> bool,
) -> Result<(), ShaderError> {
    for (index, entry) in module.entry_points.iter().enumerate() {
        let used = info.get_entry_point(index);
        for (handle, global) in module.global_variables.iter() {
            let Some(binding) = &global.binding else {
                continue;
            };
            if used[handle].is_empty() {
                continue;
            }
            if !bound(&entry.name, binding) {
                return Err(ShaderError::Layout(format!(
                    "{} uses `{}` (group {}, binding {}), which has no slot",
                    entry.name,
                    global.name.as_deref().unwrap_or("?"),
                    binding.group,
                    binding.binding
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resource_is_bound_to_slot_zero_of_its_kind() {
        let out = msl(INK_WGSL).expect("translates");
        assert!(
            out.contains("[[buffer(0)]]"),
            "the uniform buffer is at buffer 0"
        );
        assert!(
            out.contains("[[texture(0)]]"),
            "the wordmark is at texture 0"
        );
        assert!(
            out.contains("[[sampler(0)]]"),
            "its sampler is at sampler 0"
        );
        // Without a binding map naga marks each resource [[user(fake0)]] and Metal picks a slot.
        assert!(!out.contains("fake"), "no placeholder binding is left");
    }

    #[test]
    fn both_entry_points_are_written() {
        let out = msl(INK_WGSL).expect("translates");
        assert!(
            out.contains("vertex vs_mainOutput vs_main("),
            "the vertex stage"
        );
        assert!(
            out.contains("fragment fs_mainOutput fs_main("),
            "the fragment stage"
        );
    }

    #[test]
    fn the_uniform_block_is_the_documented_144_bytes() {
        let layout = uniform_layout(INK_WGSL).expect("parses");
        assert_eq!(layout.size, 144);
        assert_eq!(
            layout.drops_offset, 48,
            "the droplets follow the twelve scalars"
        );
    }

    #[test]
    fn a_resource_without_a_slot_is_refused() {
        // A fourth binding would have no slot in the map: an error, never a placeholder.
        let extra = format!(
            "{INK_WGSL}\n@group(0) @binding(3) var<uniform> extra: vec4<f32>;\n@fragment fn extra_main() -> @location(0) vec4<f32> {{ return extra; }}\n"
        );
        assert!(
            matches!(msl(&extra), Err(ShaderError::Layout(_))),
            "{:?}",
            msl(&extra)
        );
    }

    #[test]
    fn a_parse_error_names_its_line() {
        let broken = INK_WGSL.replacen(
            "let fib = fibres(frag / 180.0);",
            "let fib = fibres(frag / 180.0)",
            1,
        );
        match msl(&broken) {
            Err(ShaderError::Parse(m)) => assert!(m.contains("wgsl:"), "{m}"),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn the_hlsl_binds_each_resource_to_register_zero_of_its_kind() {
        let out = hlsl(INK_WGSL).expect("translates");
        assert!(
            out.contains("cbuffer u : register(b0)"),
            "the uniform block"
        );
        assert!(
            out.contains("Texture2D<float4> markTex : register(t0);"),
            "the wordmark"
        );
        assert!(
            out.contains("SamplerState markSamp : register(s0);"),
            "its sampler, plain"
        );
        // Direct3D 12's sampler heap cannot compile for shader model 5.0.
        assert!(!out.contains("nagaSamplerHeap"), "no sampler heap");
        assert!(!out.contains("StructuredBuffer"), "no sampler index buffer");
        assert!(!out.contains("space"), "every register in space 0");
    }

    #[test]
    fn the_hlsl_has_both_entry_points() {
        let out = hlsl(INK_WGSL).expect("translates");
        assert!(out.contains("float4 vs_main(uint vi : SV_VertexID) : SV_Position"));
        assert!(out.contains(") : SV_Target0"));
        assert!(out.contains("float4 fs_main("));
    }

    #[test]
    fn an_unexpected_sampler_heap_is_refused() {
        // The rewrite replaces exactly naga's four heap lines; anything else is an error.
        assert!(matches!(
            plain_sampler("SamplerState nagaSamplerHeap[4096]: register(s0, space0);\n"),
            Err(ShaderError::Hlsl(_))
        ));
        assert!(matches!(
            plain_sampler("float4 fs_main() : SV_Target0 { return 0; }\n"),
            Err(ShaderError::Hlsl(_))
        ));
    }

    #[test]
    fn an_hlsl_resource_without_a_register_is_refused() {
        let extra = format!(
            "{INK_WGSL}\n@group(0) @binding(3) var<uniform> extra: vec4<f32>;\n@fragment fn extra_main() -> @location(0) vec4<f32> {{ return extra; }}\n"
        );
        assert!(
            matches!(hlsl(&extra), Err(ShaderError::Layout(_))),
            "{:?}",
            hlsl(&extra)
        );
    }

    #[test]
    fn the_hlsl_file_says_where_it_comes_from() {
        let file = hlsl_file(INK_WGSL).expect("translates");
        assert!(file.starts_with("// Generated from shaders/ink.wgsl"));
        assert!(file.contains("Do not edit"));
        assert!(file.ends_with('\n'));
    }

    #[test]
    fn the_file_says_where_it_comes_from() {
        let file = msl_file(INK_WGSL).expect("translates");
        assert!(
            file.starts_with("// Generated from shaders/ink.wgsl"),
            "{}",
            &file[..80.min(file.len())]
        );
        assert!(file.contains("Do not edit"));
        assert!(file.ends_with('\n'));
    }
}
