//! Silero VAD on tract: the speech probability behind ink-audio's VAD logic (`engine-silero`).
//!
//! # Why tract
//!
//! [tract](https://github.com/sonos/tract) runs the ONNX graph in pure Rust: no native runtime,
//! no build-time download, and every crate visible to `cargo deny`. The model is the published
//! 16 kHz, opset-15 export of Silero VAD v6 (MIT; see [`SILERO_VAD_FILE`]), fetched at runtime,
//! never bundled.
//!
//! # The graph, and why it is rewritten before tract sees it
//!
//! The export takes `input` (a window with the previous window's last [`CONTEXT`] samples in
//! front of it), `state` (the recurrent state, `[2, 1, 128]`) and `sr` (the sample rate), and
//! returns the probability and the next state. It branches with ONNX `If` nodes on facts that
//! are fixed once the shapes and the rate are fixed (the rank of a tensor, whether a state was
//! given). tract types both branches of an `If` before it can fold the condition, and here the
//! branches differ in shape (a squeeze against an identity), so the stock graph does not load.
//!
//! [`resolve_ifs`] therefore asks tract's own shape analysis for each condition, with the input
//! shapes and `sr = 16000` pinned, and inlines the branch that condition takes, recursively. A
//! condition analysis cannot settle is an error, never a guess. The rewritten graph is what the
//! model computes for these inputs; nothing about the arithmetic changes.
//!
//! # Correctness without a reference runtime
//!
//! No ONNX reference runtime is part of this build, so the adapter is held to behaviour
//! (`tests/silero.rs`, `#[ignore]`, model required): high probability over AMI speech and low
//! over its pauses and over noise, state carried from window to window, a reset that restores
//! the initial state exactly, and bit-identical output across runs.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use ink_audio::vad::{SpeechProbability, VAD_WINDOW};
use ink_core::{CANONICAL_RATE, EngineError};
use tract_onnx::ops::logic::If;
use tract_onnx::pb;
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::infer::Factoid;

use crate::model_dir::ModelDir;
use crate::registry::EngineRow;
use crate::residency::Loader;
use crate::rows::SILERO_VAD_FILE;

/// Samples of the previous window the model sees in front of each new one (Silero's 16 kHz
/// context).
pub const CONTEXT: usize = 64;

/// The model's `input` length: the context, then the window.
const INPUT_LEN: usize = CONTEXT + VAD_WINDOW;

/// The recurrent state's shape: two LSTM tensors (h and c), batch 1, 128 units.
const STATE_SHAPE: [usize; 3] = [2, 1, 128];

/// The state's length in floats.
const STATE_LEN: usize = 2 * 128;

/// The input facts every load pins: `input`, `state`, and `sr` as the constant 16000.
fn input_facts() -> [InferenceFact; 3] {
    [
        f32::fact([1, INPUT_LEN]).into(),
        f32::fact(STATE_SHAPE).into(),
        InferenceFact::from(tensor0(i64::from(CANONICAL_RATE))),
    ]
}

/// A loaded Silero model: the optimised plan, shared by every [`SileroVad`] opened from it.
///
/// `Send + Sync`. Loading takes tens of milliseconds; inference is per [`SileroVad`].
#[derive(Clone)]
pub struct SileroModel {
    plan: Arc<TypedSimplePlan>,
}

impl fmt::Debug for SileroModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SileroModel")
    }
}

impl SileroModel {
    /// **Worker.** Loads the ONNX file at `path`. A missing file is
    /// [`EngineError::ModelMissing`]; a file that is not this model's graph is
    /// [`EngineError::Failed`], naming what did not match.
    pub fn load(path: &Path) -> Result<Self, EngineError> {
        if !path.is_file() {
            return Err(EngineError::ModelMissing(format!(
                "Silero VAD: no model file at {}",
                path.display()
            )));
        }
        let proto = tract_onnx::onnx()
            .proto_model_for_path(path)
            .map_err(|e| failed("reading the model", &e))?;
        Self::from_proto(proto)
    }

    /// Loads a parsed ONNX model (the file's contents).
    fn from_proto(proto: pb::ModelProto) -> Result<Self, EngineError> {
        let facts = input_facts();
        let proto = resolve_ifs(proto, &facts)?;
        let plan = with_facts(
            tract_onnx::onnx()
                .model_for_proto_model(&proto)
                .map_err(|e| failed("parsing the rewritten graph", &e))?,
            &facts,
        )?
        .into_optimized()
        .map_err(|e| failed("optimising the graph", &e))?
        .into_runnable()
        .map_err(|e| failed("planning the graph", &e))?;
        check_signature(plan.model())?;
        Ok(Self { plan })
    }

    /// A fresh speech-probability source over this model, with its own recurrent state.
    pub fn vad(&self) -> Result<SileroVad, EngineError> {
        SileroVad::new(Arc::clone(&self.plan))
    }
}

/// One stream's Silero state: implements [`SpeechProbability`].
///
/// **Worker**, one buffer or stream at a time, as the trait says. Each window allocates a few
/// small tensors (tract's inputs and outputs); none of this runs on a realtime thread.
pub struct SileroVad {
    session: TypedSimpleState,
    /// The recurrent state the next window starts from, `STATE_SHAPE` in row-major order. Held as
    /// plain floats so a reset cannot fail.
    state: [f32; STATE_LEN],
    /// The last [`CONTEXT`] samples of the previous window (zeros after a reset).
    context: [f32; CONTEXT],
}

impl fmt::Debug for SileroVad {
    // Never the state or the context: they are derived from audio.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SileroVad")
    }
}

impl SileroVad {
    fn new(plan: Arc<TypedSimplePlan>) -> Result<Self, EngineError> {
        let session =
            TypedSimpleState::new(&plan).map_err(|e| failed("opening an inference session", &e))?;
        Ok(Self {
            session,
            state: [0.0; STATE_LEN],
            context: [0.0; CONTEXT],
        })
    }
}

impl SpeechProbability for SileroVad {
    fn reset(&mut self) {
        self.state = [0.0; STATE_LEN];
        self.context = [0.0; CONTEXT];
    }

    fn probability(&mut self, window: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        let mut input = [0.0f32; INPUT_LEN];
        input[..CONTEXT].copy_from_slice(&self.context);
        input[CONTEXT..].copy_from_slice(window);
        let input = Tensor::from_shape(&[1, INPUT_LEN], &input)
            .map_err(|e| window_failed("building the input", &e))?;
        let state = Tensor::from_shape(&STATE_SHAPE, &self.state)
            .map_err(|e| window_failed("building the state", &e))?;
        let outputs = self
            .session
            .run(tvec!(
                input.into(),
                state.into(),
                tensor0(i64::from(CANONICAL_RATE)).into()
            ))
            .map_err(|e| window_failed("inference", &e))?;
        let [probability, state] = outputs.as_slice() else {
            return Err(EngineError::Failed(format!(
                "Silero VAD returned {} outputs, expected 2",
                outputs.len()
            )));
        };
        let p = scalar(probability)?;
        let next = state
            .try_as_plain_ram()
            .and_then(|v| v.as_slice::<f32>().map(<[f32]>::to_vec))
            .map_err(|e| window_failed("reading the state", &e))?;
        self.state = <[f32; STATE_LEN]>::try_from(next.as_slice()).map_err(|_| {
            EngineError::Failed(format!(
                "Silero VAD returned a state of shape {:?}, expected {STATE_SHAPE:?}",
                state.shape()
            ))
        })?;
        // The window's tail is the next window's context.
        self.context
            .copy_from_slice(&window[VAD_WINDOW - CONTEXT..]);
        Ok(p)
    }
}

/// Loads the Silero row's model for [`Residency`](crate::Residency): the row's file named
/// [`SILERO_VAD_FILE`]`.name`, installed under `dir`.
#[derive(Clone, Debug)]
pub struct SileroLoader {
    dir: ModelDir,
}

impl SileroLoader {
    /// Loads from rows installed under `dir`.
    pub fn new(dir: ModelDir) -> Self {
        Self { dir }
    }
}

impl Loader<SileroModel> for SileroLoader {
    fn load(&self, row: &EngineRow) -> Result<SileroModel, EngineError> {
        let file = row
            .files
            .iter()
            .find(|f| f.name == SILERO_VAD_FILE.name)
            .ok_or_else(|| {
                EngineError::Failed(format!(
                    "row {} has no file named {}",
                    row.id, SILERO_VAD_FILE.name
                ))
            })?;
        SileroModel::load(&self.dir.file_path(row, file))
    }
}

/// The single `f32` of a `[1, 1]` output.
fn scalar(value: &TValue) -> Result<f32, EngineError> {
    if value.len() != 1 {
        return Err(EngineError::Failed(format!(
            "Silero VAD returned a probability of shape {:?}, expected one value",
            value.shape()
        )));
    }
    value
        .try_as_plain_ram()
        .and_then(|v| v.as_slice::<f32>().map(|s| s[0]))
        .map_err(|e| window_failed("reading the probability", &e))
}

/// A failure while loading: no audio has been seen, so the whole error chain is reported.
fn failed(what: &str, e: &TractError) -> EngineError {
    EngineError::Failed(format!("Silero VAD: {what}: {e:#}"))
}

/// A failure on a window: the outermost message only (`{}`, not `{:#}`), because the chain can
/// dump tensors, and those are audio (I5).
fn window_failed(what: &str, e: &TractError) -> EngineError {
    EngineError::Failed(format!("Silero VAD: {what} failed: {e}"))
}

fn with_facts(
    model: InferenceModel,
    facts: &[InferenceFact],
) -> Result<InferenceModel, EngineError> {
    let mut model = model;
    if model.inputs.len() != facts.len() {
        return Err(EngineError::Failed(format!(
            "Silero VAD: the graph has {} inputs, expected {} (input, state, sr)",
            model.inputs.len(),
            facts.len()
        )));
    }
    for (ix, fact) in facts.iter().enumerate() {
        model = model
            .with_input_fact(ix, fact.clone())
            .map_err(|e| failed("pinning the input shapes", &e))?;
    }
    Ok(model)
}

/// Checks the optimised graph has the signature [`SileroVad`] relies on: three inputs and two
/// outputs, a `[1, 1]` probability and a state of [`STATE_SHAPE`].
fn check_signature(model: &TypedModel) -> Result<(), EngineError> {
    let outputs = model
        .output_outlets()
        .map_err(|e| failed("reading the outputs", &e))?;
    let shape = |ix: usize| -> Result<Vec<usize>, EngineError> {
        let fact = model
            .outlet_fact(outputs[ix])
            .map_err(|e| failed("reading an output", &e))?;
        fact.shape
            .as_concrete()
            .map(<[usize]>::to_vec)
            .ok_or_else(|| {
                EngineError::Failed(format!(
                    "Silero VAD: output {ix} has no fixed shape ({:?})",
                    fact.shape
                ))
            })
    };
    if model.inputs.len() != 3 || outputs.len() != 2 {
        return Err(EngineError::Failed(format!(
            "Silero VAD: the graph has {} inputs and {} outputs, expected 3 and 2",
            model.inputs.len(),
            outputs.len()
        )));
    }
    let (probability, state) = (shape(0)?, shape(1)?);
    if probability != [1, 1] || state != STATE_SHAPE {
        return Err(EngineError::Failed(format!(
            "Silero VAD: outputs have shapes {probability:?} and {state:?}, expected [1, 1] and {STATE_SHAPE:?}"
        )));
    }
    Ok(())
}

/// Replaces every ONNX `If` in `proto` whose condition is fixed by `facts` with the branch it
/// takes, recursively (branches hold `If`s too). The conditions come from tract's analysis of the
/// graph as given, with `facts` pinned on its inputs. An `If` whose condition analysis leaves
/// open, or two `If`s with one name, is an error.
pub(crate) fn resolve_ifs(
    mut proto: pb::ModelProto,
    facts: &[InferenceFact],
) -> Result<pb::ModelProto, EngineError> {
    let mut model = with_facts(
        tract_onnx::onnx()
            .model_for_proto_model(&proto)
            .map_err(|e| failed("parsing the graph", &e))?,
        facts,
    )?;
    model
        .analyse(false)
        .map_err(|e| failed("analysing the graph", &e))?;
    let mut conditions = HashMap::new();
    collect_conditions(&model, &mut conditions)?;
    let graph = proto
        .graph
        .as_mut()
        .ok_or_else(|| EngineError::Failed("Silero VAD: the model has no graph".into()))?;
    inline_ifs(graph, &conditions)?;
    Ok(proto)
}

/// Each `If` on the taken path, by node name, with the branch its condition takes.
fn collect_conditions(
    model: &InferenceModel,
    out: &mut HashMap<String, bool>,
) -> Result<(), EngineError> {
    for node in &model.nodes {
        let Some(op) = node.op_as::<If>() else {
            continue;
        };
        let condition = model
            .outlet_fact(node.inputs[0])
            .map_err(|e| failed("reading a condition", &e))?
            .value
            .concretize()
            .ok_or_else(|| {
                EngineError::Failed(format!(
                    "Silero VAD: the condition of If {:?} is not fixed by the input shapes",
                    node.name
                ))
            })?
            .cast_to_scalar::<bool>()
            .map_err(|e| failed("reading a condition", &e))?;
        if node.name.is_empty() || out.insert(node.name.clone(), condition).is_some() {
            return Err(EngineError::Failed(format!(
                "Silero VAD: If {:?} is not uniquely named",
                node.name
            )));
        }
        collect_conditions(
            if condition {
                &op.then_body
            } else {
                &op.else_body
            },
            out,
        )?;
    }
    Ok(())
}

/// Inlines the taken branch of each `If` in `graph`: the branch's nodes (its own `If`s resolved
/// first), its initializers and value hints move into `graph`, and an `Identity` per output
/// renames the branch's outputs to the `If`'s. ONNX branches read outer values by name, so the
/// moved nodes need no rewiring.
fn inline_ifs(
    graph: &mut pb::GraphProto,
    conditions: &HashMap<String, bool>,
) -> Result<(), EngineError> {
    for node in std::mem::take(&mut graph.node) {
        if node.op_type != "If" {
            graph.node.push(node);
            continue;
        }
        let taken = conditions.get(&node.name).ok_or_else(|| {
            EngineError::Failed(format!(
                "Silero VAD: no condition was found for If {:?}",
                node.name
            ))
        })?;
        let wanted = if *taken { "then_branch" } else { "else_branch" };
        let mut branch = node
            .attribute
            .iter()
            .find(|a| a.name == wanted)
            .and_then(|a| a.g.clone())
            .ok_or_else(|| {
                EngineError::Failed(format!("Silero VAD: If {:?} has no {wanted}", node.name))
            })?;
        inline_ifs(&mut branch, conditions)?;
        if branch.output.len() != node.output.len() {
            return Err(EngineError::Failed(format!(
                "Silero VAD: the {wanted} of If {:?} has {} outputs, the node {}",
                node.name,
                branch.output.len(),
                node.output.len()
            )));
        }
        graph.initializer.append(&mut branch.initializer);
        graph.value_info.append(&mut branch.value_info);
        graph.node.append(&mut branch.node);
        for (from, to) in branch.output.iter().zip(&node.output) {
            graph.node.push(pb::NodeProto {
                op_type: "Identity".into(),
                name: format!("{}/inlined/{to}", node.name),
                input: vec![from.name.clone()],
                output: vec![to.clone()],
                ..Default::default()
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The `If` rewrite on small synthetic graphs, so it is checked without the model.

    use super::*;

    fn value_info(name: &str) -> pb::ValueInfoProto {
        pb::ValueInfoProto {
            name: name.into(),
            ..Default::default()
        }
    }

    /// A float tensor input of unknown shape (the facts pin it).
    fn float_input(name: &str) -> pb::ValueInfoProto {
        pb::ValueInfoProto {
            r#type: Some(pb::TypeProto {
                value: Some(pb::type_proto::Value::TensorType(pb::type_proto::Tensor {
                    elem_type: pb::tensor_proto::DataType::Float as i32,
                    shape: None,
                })),
                ..Default::default()
            }),
            ..value_info(name)
        }
    }

    fn node(op: &str, name: &str, inputs: &[&str], outputs: &[&str]) -> pb::NodeProto {
        pb::NodeProto {
            op_type: op.into(),
            name: name.into(),
            input: inputs.iter().map(|s| s.to_string()).collect(),
            output: outputs.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn int_tensor(name: &str, dims: &[i64], values: &[i64]) -> pb::TensorProto {
        pb::TensorProto {
            name: name.into(),
            dims: dims.to_vec(),
            data_type: pb::tensor_proto::DataType::Int64 as i32,
            int64_data: values.to_vec(),
            ..Default::default()
        }
    }

    fn branch(name: &str, nodes: Vec<pb::NodeProto>, output: &str) -> pb::AttributeProto {
        pb::AttributeProto {
            name: name.into(),
            r#type: pb::attribute_proto::AttributeType::Graph as i32,
            g: Some(pb::GraphProto {
                name: format!("{name}_graph"),
                node: nodes,
                output: vec![value_info(output)],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Silero's problem in miniature: `y = If(cond, then: Unsqueeze(inner), else: -x)` over an
    /// input `x` of shape `[1, n]`, where `inner = If(cond, then: x + x, else: x)`, so the
    /// branches nest and differ in rank, as Silero's squeeze-or-identity branches do. `cond` is
    /// `Shape(x)[1] == 4`, fixed by the input shape, or, with `by_value`, `ReduceMax(x) > 0`,
    /// known only once the audio is (and then the branches keep one rank, so that analysis gets
    /// as far as the condition).
    fn model(by_value: bool) -> pb::ModelProto {
        let mut nodes = vec![];
        if by_value {
            nodes.push(node("ReduceMax", "max", &["x"], &["max"]));
            nodes.push(node("Greater", "cond", &["max", "zero"], &["cond"]));
        } else {
            nodes.push(node("Shape", "shape", &["x"], &["shape"]));
            nodes.push(node("Gather", "dim", &["shape", "one"], &["dim"]));
            nodes.push(node("Equal", "cond", &["dim", "four"], &["cond"]));
        }
        let inner = pb::NodeProto {
            attribute: vec![
                branch(
                    "then_branch",
                    vec![node("Add", "twice", &["x", "x"], &["doubled"])],
                    "doubled",
                ),
                branch(
                    "else_branch",
                    vec![node("Identity", "same", &["x"], &["same"])],
                    "same",
                ),
            ],
            ..node("If", "inner", &["cond"], &["inner_out"])
        };
        let outer = pb::NodeProto {
            attribute: vec![
                branch(
                    "then_branch",
                    vec![
                        inner,
                        if by_value {
                            node("Identity", "lift", &["inner_out"], &["lifted"])
                        } else {
                            node("Unsqueeze", "lift", &["inner_out", "axis0"], &["lifted"])
                        },
                    ],
                    "lifted",
                ),
                branch(
                    "else_branch",
                    vec![node("Neg", "negate", &["x"], &["negated"])],
                    "negated",
                ),
            ],
            ..node("If", "outer", &["cond"], &["y"])
        };
        nodes.push(outer);
        pb::ModelProto {
            ir_version: 8,
            opset_import: vec![pb::OperatorSetIdProto {
                domain: String::new(),
                version: 15,
            }],
            graph: Some(pb::GraphProto {
                name: "toy".into(),
                node: nodes,
                initializer: vec![
                    int_tensor("one", &[], &[1]),
                    int_tensor("four", &[], &[4]),
                    int_tensor("axis0", &[1], &[0]),
                    pb::TensorProto {
                        name: "zero".into(),
                        dims: vec![],
                        data_type: pb::tensor_proto::DataType::Float as i32,
                        float_data: vec![0.0],
                        ..Default::default()
                    },
                ],
                input: vec![float_input("x")],
                output: vec![value_info("y")],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn plan(proto: &pb::ModelProto, n: usize) -> TractResult<Arc<TypedSimplePlan>> {
        tract_onnx::onnx()
            .model_for_proto_model(proto)?
            .with_input_fact(0, f32::fact([1, n]).into())?
            .into_optimized()?
            .into_runnable()
    }

    /// Runs `proto` on `x` (shape `[1, n]`) and returns the output's shape and values.
    fn run(proto: &pb::ModelProto, n: usize, x: &[f32]) -> (Vec<usize>, Vec<f32>) {
        let out = plan(proto, n)
            .unwrap()
            .run(tvec!(Tensor::from_shape(&[1, n], x).unwrap().into()))
            .unwrap();
        let values = out[0]
            .try_as_plain_ram()
            .unwrap()
            .as_slice::<f32>()
            .unwrap()
            .to_vec();
        (out[0].shape().to_vec(), values)
    }

    #[test]
    fn the_stock_graph_does_not_load_which_is_why_it_is_rewritten() {
        // tract types both branches before it folds the condition; branches of different rank
        // stop it there. This is the failure the rewrite exists for.
        let err = plan(&model(false), 4).unwrap_err();
        assert!(format!("{err:#}").contains("If"), "{err:#}");
    }

    #[test]
    fn an_if_fixed_by_the_input_shape_is_replaced_by_the_branch_it_takes() {
        // Width 4: both conditions hold: y = Unsqueeze(x + x).
        let facts = [f32::fact([1, 4]).into()];
        let resolved = resolve_ifs(model(false), &facts).unwrap();
        assert!(
            !resolved
                .graph
                .as_ref()
                .unwrap()
                .node
                .iter()
                .any(|n| n.op_type == "If"),
            "an If survived the rewrite"
        );
        let (shape, y) = run(&resolved, 4, &[1.0, -2.0, 3.0, 0.5]);
        assert_eq!(shape, [1, 1, 4]);
        assert_eq!(y, [2.0, -4.0, 6.0, 1.0]);
        // Width 3: the outer condition fails: y = -x, and the inner If is never reached.
        let facts = [f32::fact([1, 3]).into()];
        let resolved = resolve_ifs(model(false), &facts).unwrap();
        let (shape, y) = run(&resolved, 3, &[1.0, -2.0, 3.0]);
        assert_eq!(shape, [1, 3]);
        assert_eq!(y, [-1.0, 2.0, -3.0]);
    }

    #[test]
    fn an_if_that_depends_on_the_audio_is_refused_not_guessed() {
        let facts = [f32::fact([1, 4]).into()];
        let err = resolve_ifs(model(true), &facts).unwrap_err();
        assert!(
            matches!(&err, EngineError::Failed(m) if m.contains("not fixed by the input shapes")),
            "{err:?}"
        );
    }

    #[test]
    fn a_missing_model_file_is_model_missing() {
        let err = SileroModel::load(Path::new("no-such-dir/silero.onnx")).unwrap_err();
        assert!(matches!(err, EngineError::ModelMissing(_)), "{err:?}");
    }

    #[test]
    fn a_file_that_is_not_an_onnx_model_is_a_failure() {
        let dir = std::env::temp_dir().join(format!("ink-engines-silero-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("garbage.onnx");
        std::fs::write(&path, b"not a model").unwrap();
        let err = SileroModel::load(&path).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(err, EngineError::Failed(_)), "{err:?}");
    }
}
