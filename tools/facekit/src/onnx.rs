//! Loading the models: the raw protobuf for the weights, and tract for
//! running them.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail};
use tract_onnx::{
    pb::{GraphProto, ModelProto, NodeProto, TensorProto},
    prelude::*,
};

/// Read the protobuf of an ONNX file.
///
/// # Errors
///
/// When the file cannot be read or parsed.
pub fn load_proto(path: &Path) -> Result<ModelProto> {
    tract_onnx::onnx()
        .proto_model_for_path(path)
        .with_context(|| format!("reading {}", path.display()))
}

/// The graph of a model.
///
/// # Errors
///
/// When the model has no graph.
pub fn graph(proto: &ModelProto) -> Result<&GraphProto> {
    proto.graph.as_ref().context("the model has no graph")
}

/// The weight tensors by name.
pub fn initializers(graph: &GraphProto) -> BTreeMap<&str, &TensorProto> {
    graph
        .initializer
        .iter()
        .map(|tensor| (tensor.name.as_str(), tensor))
        .collect()
}

/// The `f32` values of a weight tensor, whichever way the file stores them.
///
/// # Errors
///
/// When the tensor is not `f32`.
pub fn tensor_f32(tensor: &TensorProto) -> Result<Vec<f32>> {
    // 1 is the ONNX code of f32.
    if tensor.data_type != 1 {
        bail!("{}: data type {} is not f32", tensor.name, tensor.data_type);
    }
    if !tensor.float_data.is_empty() {
        return Ok(tensor.float_data.clone());
    }
    Ok(tensor
        .raw_data
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

/// The dimensions of a weight tensor as `usize`.
pub fn dims(tensor: &TensorProto) -> Vec<usize> {
    tensor.dims.iter().map(|&dim| dim.max(0) as usize).collect()
}

/// An integer attribute of a node, or `default`.
pub fn attribute_int(node: &NodeProto, name: &str, default: i64) -> i64 {
    node.attribute
        .iter()
        .find(|attribute| attribute.name == name)
        .map_or(default, |attribute| attribute.i)
}

/// One output of a run: the name it was requested by, its shape, and its
/// values in row-major order.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// The ONNX tensor name the output was requested by.
    pub name: String,
    /// The shape tract reported.
    pub shape: Vec<usize>,
    /// The values, row-major.
    pub values: Vec<f32>,
}

/// A model loaded by tract, with a fixed input shape, ready to run and to
/// report the outputs of chosen nodes.
pub struct Runner {
    /// The plan that runs the model.
    plan: std::sync::Arc<TypedSimplePlan>,
    /// The names the outputs were requested by, in output order.
    names: Vec<String>,
}

impl Runner {
    /// Load `path`, fix its input to `shape`, and make the outputs of the
    /// nodes `outputs` (given by their ONNX output tensor names) the model's
    /// outputs. An empty list keeps the model's own outputs.
    ///
    /// # Errors
    ///
    /// When tract cannot load the model, a name is unknown, or the model
    /// cannot be typed.
    pub fn new(path: &Path, shape: &[usize], outputs: &[String]) -> Result<Self> {
        let mut model = tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| format!("tract could not load {}", path.display()))?
            .with_input_fact(0, f32::fact(shape).into())
            .context("setting the input shape")?;
        let names: Vec<String> = if outputs.is_empty() {
            model
                .output_outlets()?
                .iter()
                .map(|outlet| {
                    model
                        .outlet_label(*outlet)
                        .unwrap_or(&model.node(outlet.node).name)
                        .to_string()
                })
                .collect()
        } else {
            let outlets = outputs
                .iter()
                .map(|name| {
                    model
                        .find_outlet_label(name)
                        .or_else(|| {
                            model
                                .node_by_name(name)
                                .ok()
                                .map(|node| OutletId::new(node.id, 0))
                        })
                        .with_context(|| format!("no node or tensor called {name}"))
                })
                .collect::<Result<Vec<_>>>()?;
            model.select_output_outlets(&outlets)?;
            outputs.to_vec()
        };
        // Decluttering keeps the graph readable and every requested output;
        // the optimizer could fuse across them.
        let plan = model
            .into_typed()
            .context("typing the model")?
            .into_decluttered()
            .context("decluttering the model")?
            .into_runnable()
            .context("planning the model")?;
        Ok(Self { plan, names })
    }

    /// Run the model on `input` (values in the fixed input shape) and return
    /// every `f32` output. Outputs of other types (shape arithmetic) are
    /// skipped.
    ///
    /// # Errors
    ///
    /// When the input shape does not match or tract fails.
    pub fn run(&self, shape: &[usize], input: &[f32]) -> Result<Vec<Output>> {
        let tensor = Tensor::from_shape(shape, input)?;
        let outputs = self.plan.run(tvec!(tensor.into()))?;
        let mut result = Vec::new();
        for (name, output) in self.names.iter().zip(outputs.iter()) {
            if output.datum_type() != f32::datum_type() {
                continue;
            }
            result.push(Output {
                name: name.clone(),
                shape: output.shape().to_vec(),
                values: output
                    .to_plain_array_view::<f32>()?
                    .iter()
                    .copied()
                    .collect(),
            });
        }
        Ok(result)
    }
}
