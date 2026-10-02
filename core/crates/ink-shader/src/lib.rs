//! The ink shader, written once in WGSL (`shaders/ink.wgsl`) and translated with naga into what
//! each shell compiles: the Metal Shading Language for the Mac, HLSL (shader model 5.0, for
//! Direct3D 11) for Windows.
//!
//! It also generates the shells' design tokens from `design/tokens.json` ([`tokens`]).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod tokens;

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
    /// A binding the shells rely on is missing, has another type, or another binding is used.
    Layout(String),
    /// The MSL back end refused it.
    Msl(String),
    /// The HLSL back end refused it.
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
        // Everything else as naga's CLI sets it: unchecked indexing (the shader indexes no
        // array), and loops marked as bounded.
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
/// for Direct3D 12 only). The uniform block, the one resource, sits on register `b0` in space 0
/// (see [`hlsl_binding_map`]). The shader samples no texture, so naga writes no sampler heap (a
/// Direct3D 12 binding model that shader model 5.0 cannot express).
pub fn hlsl(wgsl: &str) -> Result<String, ShaderError> {
    let (module, info) = parse(wgsl)?;
    let map = hlsl_binding_map();
    check_bindings(&module, &info, |_, binding| map.contains_key(binding))?;
    let options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_0,
        binding_map: map,
        // A resource without a register is an error (check_bindings names it), never a guess.
        fake_missing_bindings: false,
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
    Ok(out)
}

/// The register of bind group 0's one binding for HLSL: the uniform block on `b0`.
pub fn hlsl_binding_map() -> naga::back::hlsl::BindingMap {
    let mut map = naga::back::hlsl::BindingMap::new();
    map.insert(
        naga::ResourceBinding {
            group: 0,
            binding: 0,
        },
        naga::back::hlsl::BindTarget {
            register: 0,
            ..naga::back::hlsl::BindTarget::default()
        },
    );
    map
}

/// The whole generated HLSL file: a header naming its source, then [`hlsl`].
pub fn hlsl_file(wgsl: &str) -> Result<String, ShaderError> {
    let body = hlsl(wgsl)?;
    let mut out = String::from(
        "// Generated from shaders/ink.wgsl by core/crates/ink-shader (naga). Do not edit: change the\n\
         // WGSL, then run `cargo run -p ink-shader --bin ink-shader` in core/.\n\
         //\n\
         // Shader model 5.0 (Direct3D 11, compiled at run time with D3DCompile). The uniform block G\n\
         // at b0, the only resource.\n",
    );
    out.push_str(&body);
    Ok(out)
}

/// The uniform block `G` as the shells fill it: each field's name and byte offset, in order. The
/// block is [`UNIFORM_SIZE`] bytes. The header of `shaders/ink.wgsl` says what each field holds.
pub const UNIFORM_FIELDS: &[(&str, u32)] = &[
    ("res", 0),
    ("center", 8),
    ("time", 16),
    ("unit", 20),
    ("you", 24),
    ("them", 28),
    ("w", 32),
    ("dark", 48),
    ("motion", 52),
    ("pad", 56),
    ("yA", 64),
    ("yB", 80),
    ("tA", 96),
    ("tB", 112),
    ("idle", 128),
    ("ink", 144),
];

/// The uniform block's size in bytes.
pub const UNIFORM_SIZE: u32 = 160;

/// The uniform block's layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniformLayout {
    /// Its size in bytes.
    pub size: u32,
    /// Each field's name and byte offset, in order.
    pub fields: Vec<(String, u32)>,
}

/// The uniform block's layout in `wgsl`: the struct behind the uniform at group 0, binding 0, as
/// the WGSL rules lay it out. The shells fill it by these offsets.
pub fn uniform_layout(wgsl: &str) -> Result<UniformLayout, ShaderError> {
    let (module, _) = parse(wgsl)?;
    let (_, global) = module
        .global_variables
        .iter()
        .find(|(_, g)| {
            g.space == naga::AddressSpace::Uniform
                && g.binding
                    == Some(naga::ResourceBinding {
                        group: 0,
                        binding: 0,
                    })
        })
        .ok_or_else(|| ShaderError::Layout("no uniform at group 0, binding 0".into()))?;
    let naga::TypeInner::Struct { members, span } = &module.types[global.ty].inner else {
        return Err(ShaderError::Layout(
            "the uniform at group 0, binding 0 is not a struct".into(),
        ));
    };
    Ok(UniformLayout {
        size: *span,
        fields: members
            .iter()
            .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
            .collect(),
    })
}

/// The whole generated file: a header naming its source, then [`msl`].
pub fn msl_file(wgsl: &str) -> Result<String, ShaderError> {
    let body = msl(wgsl)?;
    let mut out = String::from(
        "// Generated from shaders/ink.wgsl by core/crates/ink-shader (naga). Do not edit: change the\n\
         // WGSL, then run `cargo run -p ink-shader --bin ink-shader` in core/.\n\
         //\n\
         // Fragment stage: the uniform block G at buffer 0, the only resource.\n",
    );
    out.push_str(&body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// The slot of bind group 0's one binding, the uniform block: buffer 0 in the fragment stage.
/// The vertex stage binds nothing.
pub fn binding_map() -> naga::back::msl::EntryPointResourceMap {
    use naga::back::msl::{BindTarget, EntryPointResources};
    let mut fragment = EntryPointResources::default();
    fragment.resources.insert(
        naga::ResourceBinding {
            group: 0,
            binding: 0,
        },
        BindTarget {
            buffer: Some(0),
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
    fn the_uniform_block_is_at_buffer_zero_and_nothing_else_is_bound() {
        let out = msl(INK_WGSL).expect("translates");
        assert!(
            out.contains("[[buffer(0)]]"),
            "the uniform buffer is at buffer 0"
        );
        assert!(!out.contains("[[texture("), "no texture");
        assert!(!out.contains("[[sampler("), "no sampler");
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
    fn the_uniform_block_is_the_documented_160_bytes() {
        let layout = uniform_layout(INK_WGSL).expect("parses");
        assert_eq!(layout.size, UNIFORM_SIZE);
        assert_eq!(UNIFORM_SIZE, 160);
        let documented: Vec<(String, u32)> = UNIFORM_FIELDS
            .iter()
            .map(|(name, offset)| ((*name).to_owned(), *offset))
            .collect();
        assert_eq!(layout.fields, documented);
    }

    #[test]
    fn a_resource_without_a_slot_is_refused() {
        // A second binding would have no slot in the map: an error, never a placeholder.
        let extra = format!(
            "{INK_WGSL}\n@group(0) @binding(1) var<uniform> extra: vec4<f32>;\n@fragment fn extra_main() -> @location(0) vec4<f32> {{ return extra; }}\n"
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
            "let soft = mix(0.24, 0.012, blot);",
            "let soft = mix(0.24, 0.012, blot)",
            1,
        );
        assert_ne!(broken, INK_WGSL, "the line to break is in the shader");
        match msl(&broken) {
            Err(ShaderError::Parse(m)) => assert!(m.contains("wgsl:"), "{m}"),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn the_hlsl_binds_the_uniform_block_to_b0_and_nothing_else() {
        let out = hlsl(INK_WGSL).expect("translates");
        assert!(
            out.contains("cbuffer g : register(b0)"),
            "the uniform block"
        );
        assert!(!out.contains("Texture2D"), "no texture");
        assert!(!out.contains("SamplerState"), "no sampler");
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
    fn an_hlsl_resource_without_a_register_is_refused() {
        let extra = format!(
            "{INK_WGSL}\n@group(0) @binding(1) var<uniform> extra: vec4<f32>;\n@fragment fn extra_main() -> @location(0) vec4<f32> {{ return extra; }}\n"
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
