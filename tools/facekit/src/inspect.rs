//! The `inspect` command: what is inside an ONNX model?
//!
//! The firmware does not run ONNX. It has hand-written code for each
//! operator (Conv, LayerNormalization, ...) that the two models use. So the
//! first question is: which operators are there, and with which settings?
//! This command reads the model file directly, as ONNX's protobuf
//! structures, and prints:
//!
//! - the graph inputs and outputs with their shapes,
//! - every node in graph order with its operator and the attributes that
//!   matter for a kernel (kernel size, stride, padding, groups, axes),
//! - every weight tensor with its shape, and the total parameter count,
//! - a count per operator type, and the operators that are not in the
//!   [`ALLOWED`] list.
//!
//! It also loads the model through tract, the ONNX runtime that the other
//! commands use, so a model that tract cannot run is caught here.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail};
use tract_onnx::{
    pb::{AttributeProto, GraphProto, TensorProto, ValueInfoProto, attribute_proto},
    prelude::*,
};

/// The ONNX operators that the firmware implements, or that carry no
/// arithmetic (shape plumbing that the exporter resolves at export time).
/// An operator outside this list needs a new kernel or a change of plan.
const ALLOWED: &[&str] = &[
    // Arithmetic that becomes a kernel.
    "Conv",
    "MatMul",
    "Gemm",
    "Add",
    "Sub",
    "Mul",
    "Div",
    "Relu",
    "PRelu",
    "Sigmoid",
    "Gelu",
    "Erf",
    "Softmax",
    "LayerNormalization",
    "ReduceMean",
    "GlobalAveragePool",
    "AveragePool",
    "MaxPool",
    "Pow",
    "Sqrt",
    "Exp",
    "Resize",
    // Data movement that becomes an index calculation or nothing.
    "Concat",
    "Split",
    "Slice",
    "Reshape",
    "Transpose",
    "Flatten",
    "Squeeze",
    "Unsqueeze",
    "Gather",
    "Shape",
    "Constant",
    "ConstantOfShape",
    "Cast",
    "Identity",
    "Expand",
    "Where",
    "Equal",
    "ReduceL2",
    "Clip",
];

/// Operators that only appear because the exporter of the model computed a
/// constant at run time, for example the positional encoding of EdgeFace,
/// which depends on the input size alone. The `export` command evaluates
/// them once and stores the result as a constant tensor, so the firmware
/// never sees them.
const FOLDED: &[&str] = &["Sin", "Cos", "CumSum", "Not", "Range", "Tile"];

/// Attributes that decide how a kernel is written. Others are printed only
/// when `--strict` is not given, to keep the listing readable.
const KEY_ATTRIBUTES: &[&str] = &[
    "kernel_shape",
    "strides",
    "pads",
    "dilations",
    "group",
    "axis",
    "axes",
    "epsilon",
    "perm",
    "alpha",
    "beta",
    "transB",
    "keepdims",
    "split",
    "approximate",
    "mode",
];

/// Read `model`, print its structure, and check the operators.
///
/// # Errors
///
/// When the file cannot be read or parsed, or, with `strict`, when the model
/// uses an operator outside [`ALLOWED`].
pub fn run(model: &Path, strict: bool, input_shape: Option<&[usize]>) -> Result<()> {
    let proto = tract_onnx::onnx()
        .proto_model_for_path(model)
        .with_context(|| format!("reading {}", model.display()))?;
    let graph = proto.graph.as_ref().context("the model has no graph")?;

    println!("# {}", model.display());
    println!(
        "ir_version {}, producer {} {}",
        proto.ir_version, proto.producer_name, proto.producer_version
    );
    for opset in &proto.opset_import {
        println!("opset {} {}", opset.domain, opset.version);
    }

    let initializers: BTreeMap<&str, &TensorProto> = graph
        .initializer
        .iter()
        .map(|tensor| (tensor.name.as_str(), tensor))
        .collect();

    println!("\n## Inputs");
    for input in &graph.input {
        if !initializers.contains_key(input.name.as_str()) {
            println!("{}", describe_value(input));
        }
    }
    println!("\n## Outputs");
    for output in &graph.output {
        println!("{}", describe_value(output));
    }

    println!("\n## Nodes ({})", graph.node.len());
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &graph.node {
        *counts.entry(node.op_type.as_str()).or_default() += 1;
        let weights: Vec<String> = node
            .input
            .iter()
            .filter_map(|name| initializers.get(name.as_str()))
            .map(|tensor| format!("{}{:?}", tensor.name, tensor.dims))
            .collect();
        let attributes: Vec<String> = node
            .attribute
            .iter()
            .filter(|attribute| !strict || KEY_ATTRIBUTES.contains(&attribute.name.as_str()))
            .map(describe_attribute)
            .collect();
        println!(
            "{:<32} {:<20} in={:?} out={:?}{}{}",
            node.name,
            node.op_type,
            node.input,
            node.output,
            if weights.is_empty() {
                String::new()
            } else {
                format!(" weights={}", weights.join(", "))
            },
            if attributes.is_empty() {
                String::new()
            } else {
                format!(" {}", attributes.join(" "))
            },
        );
    }

    println!("\n## Weights");
    let mut parameters: u64 = 0;
    for tensor in &graph.initializer {
        let count: u64 = tensor.dims.iter().map(|&dim| dim.max(0) as u64).product();
        parameters += count;
        println!(
            "{:<48} {:?} {} = {}",
            tensor.name,
            tensor.dims,
            data_type_name(tensor.data_type),
            count
        );
    }
    println!(
        "total parameters: {parameters} ({:.2} MB as int8, {:.2} MB as f32)",
        parameters as f64 / 1e6,
        parameters as f64 * 4.0 / 1e6
    );

    println!("\n## Operator counts");
    for (op, count) in &counts {
        let mark = if ALLOWED.contains(op) {
            ""
        } else if FOLDED.contains(op) {
            "  <-- folded to a constant by export"
        } else {
            "  <-- not implemented"
        };
        println!("{op:<24} {count}{mark}");
    }
    let unknown: Vec<&str> = counts
        .keys()
        .copied()
        .filter(|op| !ALLOWED.contains(op) && !FOLDED.contains(op))
        .collect();

    println!("\n## tract");
    check_with_tract(model, graph, input_shape)?;

    if unknown.is_empty() {
        println!("\nall operators are in the allowed list");
    } else if strict {
        bail!("operators outside the allowed list: {}", unknown.join(", "));
    } else {
        println!(
            "\noperators outside the allowed list: {}",
            unknown.join(", ")
        );
    }
    Ok(())
}

/// Load the model with tract and print what it makes of the inputs and
/// outputs. A dynamic input size is fixed to the shape given in the file's
/// input, or left symbolic when there is none.
fn check_with_tract(model: &Path, graph: &GraphProto, input_shape: Option<&[usize]>) -> Result<()> {
    let mut inference = tract_onnx::onnx()
        .model_for_path(model)
        .context("tract could not load the model")?;
    if let Some(shape) = input_shape {
        inference = inference
            .with_input_fact(0, f32::fact(shape).into())
            .context("setting the input shape")?;
    }
    for (index, outlet) in inference.input_outlets()?.iter().enumerate() {
        let fact = inference.outlet_fact(*outlet)?;
        println!("input {index}: {fact:?}");
    }
    match inference.clone().into_typed() {
        Ok(typed) => {
            let optimized = typed
                .into_optimized()
                .context("tract could not optimize the model")?;
            println!(
                "tract typed and optimized the model: {} nodes",
                optimized.nodes().len()
            );
            for (index, outlet) in optimized.output_outlets()?.iter().enumerate() {
                println!("output {index}: {:?}", optimized.outlet_fact(*outlet)?);
            }
        }
        Err(error) => {
            println!(
                "tract could not infer all shapes ({error}); the model has {} graph inputs, later commands must fix the input shape",
                graph.input.len()
            );
        }
    }
    Ok(())
}

/// `name: type[shape]` of a graph input or output.
fn describe_value(value: &ValueInfoProto) -> String {
    use tract_onnx::pb::{tensor_shape_proto::dimension, type_proto};
    let Some(type_proto::Value::TensorType(tensor)) =
        value.r#type.as_ref().and_then(|t| t.value.as_ref())
    else {
        return format!("{}: (not a tensor)", value.name);
    };
    let dims: Vec<String> = tensor
        .shape
        .as_ref()
        .map(|shape| {
            shape
                .dim
                .iter()
                .map(|dim| match &dim.value {
                    Some(dimension::Value::DimValue(value)) => value.to_string(),
                    Some(dimension::Value::DimParam(name)) => name.clone(),
                    None => "?".to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "{}: {}[{}]",
        value.name,
        data_type_name(tensor.elem_type),
        dims.join(",")
    )
}

/// `name=value` of a node attribute.
fn describe_attribute(attribute: &AttributeProto) -> String {
    let value = match attribute_proto::AttributeType::try_from(attribute.r#type) {
        Ok(attribute_proto::AttributeType::Float) => attribute.f.to_string(),
        Ok(attribute_proto::AttributeType::Int) => attribute.i.to_string(),
        Ok(attribute_proto::AttributeType::String) => {
            String::from_utf8_lossy(&attribute.s).into_owned()
        }
        Ok(attribute_proto::AttributeType::Floats) => format!("{:?}", attribute.floats),
        Ok(attribute_proto::AttributeType::Ints) => format!("{:?}", attribute.ints),
        Ok(attribute_proto::AttributeType::Tensor) => attribute
            .t
            .as_ref()
            .map(|tensor| format!("tensor{:?}", tensor.dims))
            .unwrap_or_default(),
        _ => "...".to_string(),
    };
    format!("{}={}", attribute.name, value)
}

/// The name of an ONNX element type.
fn data_type_name(data_type: i32) -> &'static str {
    match data_type {
        1 => "f32",
        2 => "u8",
        3 => "i8",
        6 => "i32",
        7 => "i64",
        9 => "bool",
        10 => "f16",
        11 => "f64",
        _ => "?",
    }
}
