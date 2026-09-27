//! The `export` command: the weights of a model as one FKB1 file, arranged
//! the way the firmware's kernels want them.
//!
//! The firmware keeps activations channels-last (`[rows][columns][channels]`).
//! So the weights are stored so that the innermost loop of every kernel
//! runs over contiguous memory:
//!
//! | ONNX operator | ONNX layout | stored as | layout string |
//! | --- | --- | --- | --- |
//! | `Conv`, one group | `[O][I][kh][kw]` | `[O][kh][kw][I]` | `OHWI` |
//! | `Conv`, depthwise | `[C][1][kh][kw]` | `[kh][kw][C]` | `HWC` |
//! | `MatMul` weight | `[I][O]` | `[O][I]` | `OI` |
//! | `Gemm` weight | `[O][I]` (transB) | `[O][I]` | `OI` |
//! | biases, norms, layer scales | `[C]` | `[C]` | `C` |
//!
//! Tensors that only exist because PyTorch computes a constant at run
//! time are evaluated once here and stored as constants: the positional
//! encodings of EdgeFace's attention blocks, `[H][W][C]`.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tract_onnx::pb::{NodeProto, TensorProto};

use crate::{
    blob::Writer,
    names,
    onnx::{self, Runner},
};

/// Write `model`'s weights to `out` (and a listing to `out` + `.txt`).
///
/// # Errors
///
/// When the model cannot be read, uses a weight in a way this tool does not
/// understand, or the files cannot be written.
pub fn run(model: &Path, input_shape: &[usize], out: &Path) -> Result<()> {
    let proto = onnx::load_proto(model)?;
    let graph = onnx::graph(&proto)?;
    let initializers = onnx::initializers(graph);

    let mut writer = Writer::new();
    let mut consumed = BTreeSet::new();
    let mut folded_nodes = Vec::new();

    for node in &graph.node {
        if node.op_type == "Conv" && node.name.contains("token_projection") {
            folded_nodes.push(node);
        }
        if node.op_type == "Resize" {
            check_resize(node, &initializers)?;
            consumed.extend(node.input.iter().map(String::as_str));
            continue;
        }
        for (index, input) in node.input.iter().enumerate() {
            let Some(tensor) = initializers.get(input.as_str()) else {
                continue;
            };
            // Shape-only inputs (Reshape shapes, Slice indices) are int64
            // and are often shared between nodes. They are not weights.
            // 1 is the ONNX code of f32.
            if tensor.data_type != 1 {
                consumed.insert(input.as_str());
                continue;
            }
            if !consumed.insert(input.as_str()) {
                bail!("weight {input} is used by more than one node");
            }
            export_parameter(&mut writer, node, index, tensor)?;
        }
    }

    for tensor in &graph.initializer {
        if !consumed.contains(tensor.name.as_str()) {
            println!(
                "note: weight {} is not used by any node; skipped",
                tensor.name
            );
        }
    }

    if !folded_nodes.is_empty() {
        fold_constants(&mut writer, model, input_shape, &folded_nodes)?;
    }

    let manifest = writer.manifest();
    let bytes = writer.finish();
    fs::write(out, &bytes).with_context(|| format!("writing {}", out.display()))?;
    let listing: PathBuf = out.with_extension("txt");
    fs::write(&listing, &manifest).with_context(|| format!("writing {}", listing.display()))?;

    print!("{manifest}");
    println!(
        "wrote {} ({} bytes, sha256 {:x}) and {}",
        out.display(),
        bytes.len(),
        Sha256::digest(&bytes),
        listing.display()
    );
    Ok(())
}

/// Store the weight `tensor`, which is input `index` of `node`, in the
/// layout the firmware wants for that operator.
fn export_parameter(
    writer: &mut Writer,
    node: &NodeProto,
    index: usize,
    tensor: &TensorProto,
) -> Result<()> {
    let name = names::parameter(&tensor.name, &node.name, index);
    let dims = onnx::dims(tensor);
    let values = onnx::tensor_f32(tensor)?;
    match (node.op_type.as_str(), index) {
        ("Conv", 1) => {
            let [out_channels, in_per_group, kernel_rows, kernel_columns] = dims[..] else {
                bail!("{name}: a convolution weight must have 4 dimensions, has {dims:?}");
            };
            let group = onnx::attribute_int(node, "group", 1) as usize;
            if group == 1 {
                // [O][I][kh][kw] -> [O][kh][kw][I]
                let mut arranged = vec![0.0f32; values.len()];
                for o in 0..out_channels {
                    for i in 0..in_per_group {
                        for kh in 0..kernel_rows {
                            for kw in 0..kernel_columns {
                                let source = ((o * in_per_group + i) * kernel_rows + kh)
                                    * kernel_columns
                                    + kw;
                                let target = ((o * kernel_rows + kh) * kernel_columns + kw)
                                    * in_per_group
                                    + i;
                                arranged[target] = values[source];
                            }
                        }
                    }
                }
                writer.add_f32(
                    &name,
                    "OHWI",
                    &[out_channels, kernel_rows, kernel_columns, in_per_group],
                    &arranged,
                )
            } else if group == out_channels && in_per_group == 1 {
                // [C][1][kh][kw] -> [kh][kw][C]
                let mut arranged = vec![0.0f32; values.len()];
                for c in 0..out_channels {
                    for kh in 0..kernel_rows {
                        for kw in 0..kernel_columns {
                            let source = (c * kernel_rows + kh) * kernel_columns + kw;
                            let target = (kh * kernel_columns + kw) * out_channels + c;
                            arranged[target] = values[source];
                        }
                    }
                }
                writer.add_f32(
                    &name,
                    "HWC",
                    &[kernel_rows, kernel_columns, out_channels],
                    &arranged,
                )
            } else {
                bail!("{name}: grouped convolution with {group} groups is not supported");
            }
        }
        ("MatMul", 1) => {
            let [inputs, outputs] = dims[..] else {
                bail!("{name}: a MatMul weight must have 2 dimensions, has {dims:?}");
            };
            let transposed = transpose(&values, inputs, outputs);
            writer.add_f32(&name, "OI", &[outputs, inputs], &transposed)
        }
        ("Gemm", 1) => {
            let [rows, columns] = dims[..] else {
                bail!("{name}: a Gemm weight must have 2 dimensions, has {dims:?}");
            };
            if onnx::attribute_int(node, "transB", 0) == 1 {
                writer.add_f32(&name, "OI", &[rows, columns], &values)
            } else {
                writer.add_f32(
                    &name,
                    "OI",
                    &[columns, rows],
                    &transpose(&values, rows, columns),
                )
            }
        }
        ("Conv" | "Gemm", 2) => writer.add_f32(&name, "O", &dims, &values),
        ("LayerNormalization", _) | ("Add" | "Mul" | "Sub" | "Div" | "PRelu", _) => {
            // Per-channel vectors, sometimes with extra 1-sized dimensions
            // (the attention temperature is [heads, 1, 1]).
            let squeezed: Vec<usize> = dims.iter().copied().filter(|&dim| dim != 1).collect();
            let layout = if squeezed.is_empty() { "S" } else { "C" };
            writer.add_f32(&name, layout, &squeezed, &values)
        }
        _ => {
            println!(
                "note: {name} is input {index} of {}; stored unchanged",
                node.op_type
            );
            writer.add_f32(&name, "RAW", &dims, &values)
        }
    }
}

/// YuNet's `Resize` nodes double the rows and columns with nearest-neighbour
/// sampling. The firmware hard-codes that, so make sure the model agrees:
/// the scales input must be `[1, 1, 2, 2]`. The `roi` input is empty and
/// shared by both nodes; neither is a weight.
fn check_resize(
    node: &NodeProto,
    initializers: &std::collections::BTreeMap<&str, &TensorProto>,
) -> Result<()> {
    let mode = node
        .attribute
        .iter()
        .find(|attribute| attribute.name == "mode")
        .map(|attribute| String::from_utf8_lossy(&attribute.s).into_owned())
        .unwrap_or_default();
    let scales = node
        .input
        .get(2)
        .and_then(|name| initializers.get(name.as_str()))
        .map(|tensor| onnx::tensor_f32(tensor))
        .transpose()?
        .unwrap_or_default();
    if mode != "nearest" || scales != [1.0, 1.0, 2.0, 2.0] {
        bail!(
            "{}: expected a nearest x2 Resize, found mode {mode:?} scales {scales:?}",
            node.name
        );
    }
    println!("note: {} is a nearest x2 upsample; no weights", node.name);
    Ok(())
}

/// `values` as a `rows` x `columns` matrix, transposed to `columns` x `rows`.
fn transpose(values: &[f32], rows: usize, columns: usize) -> Vec<f32> {
    let mut transposed = vec![0.0f32; values.len()];
    for row in 0..rows {
        for column in 0..columns {
            transposed[column * rows + row] = values[row * columns + column];
        }
    }
    transposed
}

/// Evaluate the positional-encoding convolutions once and store their
/// outputs as `<block>.pos_embd.constant`, `[H][W][C]`.
fn fold_constants(
    writer: &mut Writer,
    model: &Path,
    input_shape: &[usize],
    nodes: &[&NodeProto],
) -> Result<()> {
    let outputs: Vec<String> = nodes.iter().map(|node| node.output[0].clone()).collect();
    let runner = Runner::new(model, input_shape, &outputs)?;
    let elements: usize = input_shape.iter().product();
    let results = runner.run(input_shape, &vec![0.0f32; elements])?;
    if results.len() != nodes.len() {
        bail!(
            "expected {} folded outputs, tract returned {}",
            nodes.len(),
            results.len()
        );
    }
    for (node, output) in nodes.iter().zip(results) {
        let (shape, values) = (output.shape, output.values);
        let [1, channels, rows, columns] = shape[..] else {
            bail!(
                "{}: expected an output of shape [1, C, H, W], got {shape:?}",
                node.name
            );
        };
        let mut arranged = vec![0.0f32; values.len()];
        for c in 0..channels {
            for h in 0..rows {
                for w in 0..columns {
                    arranged[(h * columns + w) * channels + c] =
                        values[(c * rows + h) * columns + w];
                }
            }
        }
        let module = names::module(&node.name);
        let block = module.strip_suffix(".token_projection").unwrap_or(&module);
        writer.add_f32(
            &format!("{block}.constant"),
            "HWC",
            &[rows, columns, channels],
            &arranged,
        )?;
        println!(
            "folded {} into {block}.constant [{rows}, {columns}, {channels}]",
            node.name
        );
    }
    Ok(())
}
