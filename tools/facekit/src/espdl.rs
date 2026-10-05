//! Espressif's `.espdl` model files: reading them, running them as they
//! stand (the reference), and turning MFN_S8_V1 into the FKB1 file the
//! firmware runs (`import-espdl`).
//!
//! # The file
//!
//! A 16-byte header (`EDL2`, a mode word that must be 0 for an
//! unencrypted model, the length of what follows, padding), then a
//! FlatBuffer by the schema `esp-dl/fbs_loader/espdl.fbs` (MIT): an ONNX
//! graph whose tensors are `int8` with one power-of-two scale each, its
//! exponent stored next to it. Espressif's own loader is a closed
//! library, so this module reads the FlatBuffer by hand; it needs only a
//! few fields.
//!
//! # The reference
//!
//! [`Graph::run`] interprets the graph node by node in plain integer
//! arithmetic: what the firmware's `nn::mfn` must compute too, by a path
//! that shares no code with it. The arithmetic (each result rounded half
//! up and saturated to `i8`; a PReLU on the rounded convolution output,
//! as ESP-DL's separate layer does) scored 99.27 % on LFW with the
//! firmware's alignment.
//!
//! # The import
//!
//! [`import`] maps the graph onto the layers `nn::mfn` expects and
//! checks every assumption the kernels make: that an `Add` or a `Concat`
//! keeps the scale of its inputs, that a PReLU's positive side shifts by
//! 0 or 1, that every shift is a right shift. ESP-DL splits three layers
//! into halves with scales of their own and concatenates them; each pair
//! becomes one layer whose groups of sixteen channels carry their own
//! shifts. For every layer the file holds:
//!
//! | Name | Type | Content |
//! | --- | --- | --- |
//! | `<layer>.weight` | `i8` `[groups][taps][16]` | ESP-DL's `N16HWC16` order; the stem's 27 taps padded to 32 |
//! | `<layer>.bias` | `i32` `[channels]` | in product units |
//! | `<layer>.shift` | `i32` `[groups]` | the right shift to the output's units |
//! | `<layer>.alpha` | `i8` `[channels]` | the PReLU's slopes, when it has one |
//! | `<layer>.prelu` | `i32` `[groups][2]` | the PReLU's positive and negative shifts |
//!
//! and `input.exponent` (`i32`, the input's power-of-two exponent).

use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use hack_and_hike_vision::blob::DataType;

use crate::blob::{Tensor as BlobTensor, Writer};

/// A little-endian view of the FlatBuffer.
struct Buf<'a>(&'a [u8]);

impl Buf<'_> {
    fn u8(&self, at: usize) -> Result<u8> {
        self.0.get(at).copied().context("past the end of the model")
    }

    fn u16(&self, at: usize) -> Result<u16> {
        Ok(u16::from_le_bytes([self.u8(at)?, self.u8(at + 1)?]))
    }

    fn u32(&self, at: usize) -> Result<u32> {
        let b = self
            .0
            .get(at..at + 4)
            .context("past the end of the model")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&self, at: usize) -> Result<i32> {
        Ok(self.u32(at)? as i32)
    }

    fn i64(&self, at: usize) -> Result<i64> {
        let b = self
            .0
            .get(at..at + 8)
            .context("past the end of the model")?;
        Ok(i64::from_le_bytes(b.try_into()?))
    }
}

/// A FlatBuffer table: its position and its vtable.
#[derive(Clone, Copy)]
struct Table<'a> {
    buf: &'a Buf<'a>,
    pos: usize,
    vtable: usize,
    vtable_len: usize,
}

impl<'a> Table<'a> {
    fn at(buf: &'a Buf<'a>, pos: usize) -> Result<Self> {
        let vtable = (pos as i64 - i64::from(buf.i32(pos)?)) as usize;
        Ok(Self {
            buf,
            pos,
            vtable,
            vtable_len: usize::from(buf.u16(vtable)?),
        })
    }

    /// The offset of field `index` inside the table, 0 when absent.
    fn field(&self, index: usize) -> Result<usize> {
        let at = 4 + 2 * index;
        Ok(if at < self.vtable_len {
            usize::from(self.buf.u16(self.vtable + at)?)
        } else {
            0
        })
    }

    fn i32_or(&self, index: usize, default: i32) -> Result<i32> {
        match self.field(index)? {
            0 => Ok(default),
            off => self.buf.i32(self.pos + off),
        }
    }

    fn u8_or(&self, index: usize, default: u8) -> Result<u8> {
        match self.field(index)? {
            0 => Ok(default),
            off => self.buf.u8(self.pos + off),
        }
    }

    fn i64_or(&self, index: usize, default: i64) -> Result<i64> {
        match self.field(index)? {
            0 => Ok(default),
            off => self.buf.i64(self.pos + off),
        }
    }

    /// Where the object of offset field `index` starts.
    fn target(&self, index: usize) -> Result<Option<usize>> {
        Ok(match self.field(index)? {
            0 => None,
            off => {
                let at = self.pos + off;
                Some(at + self.buf.u32(at)? as usize)
            }
        })
    }

    fn table(&self, index: usize) -> Result<Option<Table<'a>>> {
        self.target(index)?
            .map(|at| Table::at(self.buf, at))
            .transpose()
    }

    fn string(&self, index: usize) -> Result<String> {
        Ok(match self.target(index)? {
            None => String::new(),
            Some(at) => {
                let len = self.buf.u32(at)? as usize;
                let bytes = self.buf.0.get(at + 4..at + 4 + len).context("string")?;
                String::from_utf8_lossy(bytes).into_owned()
            }
        })
    }

    /// The length and first element of vector field `index`.
    fn vector(&self, index: usize) -> Result<(usize, usize)> {
        Ok(match self.target(index)? {
            None => (0, 0),
            Some(at) => (self.buf.u32(at)? as usize, at + 4),
        })
    }

    fn tables(&self, index: usize) -> Result<Vec<Table<'a>>> {
        let (len, first) = self.vector(index)?;
        (0..len)
            .map(|k| {
                let at = first + 4 * k;
                Table::at(self.buf, at + self.buf.u32(at)? as usize)
            })
            .collect()
    }

    fn strings(&self, index: usize) -> Result<Vec<String>> {
        let (len, first) = self.vector(index)?;
        (0..len)
            .map(|k| {
                let at = first + 4 * k;
                let at = at + self.buf.u32(at)? as usize;
                let n = self.buf.u32(at)? as usize;
                let bytes = self.buf.0.get(at + 4..at + 4 + n).context("string")?;
                Ok(String::from_utf8_lossy(bytes).into_owned())
            })
            .collect()
    }

    fn i64s(&self, index: usize) -> Result<Vec<i64>> {
        let (len, first) = self.vector(index)?;
        (0..len).map(|k| self.buf.i64(first + 8 * k)).collect()
    }

    fn bytes(&self, index: usize, element: usize) -> Result<Vec<u8>> {
        let (len, first) = self.vector(index)?;
        Ok(self
            .buf
            .0
            .get(first..first + len * element)
            .context("vector")?
            .to_vec())
    }
}

/// A tensor's element type, as far as MFN_S8_V1 uses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `int8`.
    I8,
    /// `int32`.
    I32,
}

/// A constant tensor of the graph.
#[derive(Clone, Debug)]
pub struct Constant {
    /// The dimensions, as ESP-DL lists them (`[kh, kw, ci, co]` for a
    /// convolution's weights).
    pub dims: Vec<usize>,
    /// The power-of-two exponent of its scale.
    pub exponent: i32,
    /// The raw little-endian values.
    pub raw: Vec<u8>,
    /// ESP-DL's note on the layout, such as `layout ==> N16HWC16`.
    pub layout: String,
}

impl Constant {
    /// The values as `i8`.
    pub fn i8s(&self) -> Vec<i8> {
        self.raw.iter().map(|&b| b as i8).collect()
    }

    /// The values as `i32`.
    pub fn i32s(&self) -> Vec<i32> {
        self.raw
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
}

/// An attribute value.
#[derive(Clone, Debug)]
pub enum Attribute {
    /// An integer.
    Int(i64),
    /// A list of integers.
    Ints(Vec<i64>),
    /// A string.
    Text(String),
    /// Anything else, unused here.
    Other,
}

/// One node of the graph.
#[derive(Clone, Debug)]
pub struct Node {
    /// The ONNX operator.
    pub op: String,
    /// The input tensors' names.
    pub inputs: Vec<String>,
    /// The output tensors' names.
    pub outputs: Vec<String>,
    /// The attributes.
    pub attributes: HashMap<String, Attribute>,
}

impl Node {
    fn int(&self, name: &str) -> Result<i64> {
        match self.attributes.get(name) {
            Some(Attribute::Int(v)) => Ok(*v),
            _ => bail!("{} has no integer attribute {name}", self.op),
        }
    }

    fn ints(&self, name: &str) -> Result<Vec<i64>> {
        match self.attributes.get(name) {
            Some(Attribute::Ints(v)) => Ok(v.clone()),
            _ => bail!("{} has no attribute {name}", self.op),
        }
    }
}

/// An activation tensor: its shape and exponent.
#[derive(Clone, Debug)]
pub struct Value {
    /// `[1, height, width, channels]`.
    pub shape: Vec<usize>,
    /// The power-of-two exponent of its scale.
    pub exponent: i32,
}

/// The whole model.
#[derive(Clone, Debug)]
pub struct Graph {
    /// The nodes, in the order they run.
    pub nodes: Vec<Node>,
    /// The constants by name.
    pub constants: HashMap<String, Constant>,
    /// The activations by name, inputs and outputs included.
    pub values: HashMap<String, Value>,
    /// The input's name.
    pub input: String,
    /// The output's name.
    pub output: String,
}

/// The value info of a `ValueInfo` table.
fn value(table: &Table<'_>) -> Result<(String, Value)> {
    let name = table.string(0)?;
    let exponents = table.i64s(3)?;
    let mut shape = Vec::new();
    if let Some(type_info) = table.table(1)?
        && type_info.u8_or(0, 0)? == 1
        && let Some(tensor_type) = type_info.table(1)?
        && let Some(tensor_shape) = tensor_type.table(1)?
    {
        for dim in tensor_shape.tables(0)? {
            if let Some(dim_value) = dim.table(0)? {
                shape.push(dim_value.i64_or(1, 0)? as usize);
            }
        }
    }
    let exponent = *exponents.first().context("a value without an exponent")? as i32;
    Ok((name, Value { shape, exponent }))
}

impl Graph {
    /// Read an `.espdl` file.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is encrypted, or does not parse.
    pub fn read(path: &Path) -> Result<Self> {
        let data = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        ensure!(
            data.len() > 16 && &data[..4] == b"EDL2",
            "not an EDL2 model"
        );
        let mode = u32::from_le_bytes(data[4..8].try_into()?);
        ensure!(mode == 0, "the model is encrypted (mode {mode})");
        let len = u32::from_le_bytes(data[8..12].try_into()?) as usize;
        let body = data.get(16..16 + len).context("the model is cut short")?;
        let buf = Buf(body);
        let model = Table::at(&buf, buf.u32(0)? as usize)?;
        let graph = model.table(7)?.context("a model without a graph")?;

        let mut constants = HashMap::new();
        for tensor in graph.tables(2)? {
            let name = tensor.string(6)?;
            let dims: Vec<usize> = tensor.i64s(0)?.iter().map(|&d| d as usize).collect();
            let kind = match tensor.i32_or(1, 0)? {
                3 => Kind::I8,
                6 => Kind::I32,
                other => bail!("constant {name} has element type {other}"),
            };
            let size = match kind {
                Kind::I8 => 1,
                Kind::I32 => 4,
            };
            let count: usize = dims.iter().product();
            let mut raw = tensor.bytes(8, 16)?;
            ensure!(raw.len() >= count * size, "constant {name} is cut short");
            raw.truncate(count * size);
            let exponent = *tensor.i64s(13)?.first().context("no exponent")? as i32;
            let layout = tensor.string(7)?;
            constants.insert(
                name,
                Constant {
                    dims,
                    exponent,
                    raw,
                    layout,
                },
            );
        }

        let mut values = HashMap::new();
        let inputs = graph.tables(4)?;
        let outputs = graph.tables(5)?;
        for table in inputs.iter().chain(&outputs).chain(&graph.tables(6)?) {
            let (name, info) = value(table)?;
            values.insert(name, info);
        }
        let input = value(inputs.first().context("no input")?)?.0;
        let output = value(outputs.first().context("no output")?)?.0;

        let mut nodes = Vec::new();
        for node in graph.tables(0)? {
            let mut attributes = HashMap::new();
            for attribute in node.tables(5)? {
                let name = attribute.string(0)?;
                let value = match attribute.i32_or(3, 0)? {
                    2 => Attribute::Int(attribute.i64_or(5, 0)?),
                    7 => Attribute::Ints(attribute.i64s(11)?),
                    3 => Attribute::Text(
                        String::from_utf8_lossy(&attribute.bytes(6, 1)?).into_owned(),
                    ),
                    _ => Attribute::Other,
                };
                attributes.insert(name, value);
            }
            nodes.push(Node {
                op: node.string(3)?,
                inputs: node.strings(0)?,
                outputs: node.strings(1)?,
                attributes,
            });
        }
        Ok(Self {
            nodes,
            constants,
            values,
            input,
            output,
        })
    }

    fn constant(&self, name: &str) -> Result<&Constant> {
        self.constants
            .get(name)
            .with_context(|| format!("no constant {name}"))
    }

    fn exponent(&self, name: &str) -> Result<i32> {
        Ok(self
            .values
            .get(name)
            .with_context(|| format!("no value {name}"))?
            .exponent)
    }

    /// The input's exponent.
    ///
    /// # Errors
    ///
    /// When the graph lacks it.
    pub fn input_exponent(&self) -> Result<i32> {
        self.exponent(&self.input)
    }

    /// Run the graph on `input` (`112 x 112 x 3`, already quantized with
    /// [`Graph::input_exponent`]) and return the output's values.
    ///
    /// # Errors
    ///
    /// When the graph uses something this interpreter does not know.
    pub fn run(&self, input: &[i8]) -> Result<Vec<i8>> {
        let shape = &self.values[&self.input].shape;
        let mut maps: HashMap<&str, Map> = HashMap::new();
        maps.insert(
            &self.input,
            Map {
                h: shape[1],
                w: shape[2],
                c: shape[3],
                data: input.to_vec(),
            },
        );
        for node in &self.nodes {
            let out = node.outputs.first().context("a node without output")?;
            let out_exp = self.exponent(out)?;
            let result = match node.op.as_str() {
                "Conv" => self.conv(node, &maps[node.inputs[0].as_str()], out_exp)?,
                "PRelu" => {
                    let x = &maps[node.inputs[0].as_str()];
                    let alpha = self.constant(&node.inputs[1])?;
                    let in_exp = self.exponent(&node.inputs[0])?;
                    let slopes = alpha.i8s();
                    let mut data = Vec::with_capacity(x.data.len());
                    for (i, &v) in x.data.iter().enumerate() {
                        let v = i64::from(v);
                        data.push(if v >= 0 {
                            requantize(v, in_exp - out_exp)
                        } else {
                            requantize(
                                v * i64::from(slopes[i % x.c]),
                                in_exp + alpha.exponent - out_exp,
                            )
                        });
                    }
                    Map { data, ..x.clone() }
                }
                "Add" => {
                    let (a, b) = (
                        &maps[node.inputs[0].as_str()],
                        &maps[node.inputs[1].as_str()],
                    );
                    let (ea, eb) = (
                        self.exponent(&node.inputs[0])?,
                        self.exponent(&node.inputs[1])?,
                    );
                    let e = ea.min(eb);
                    let data = a
                        .data
                        .iter()
                        .zip(&b.data)
                        .map(|(&x, &y)| {
                            let sum = (i64::from(x) << (ea - e)) + (i64::from(y) << (eb - e));
                            requantize(sum, e - out_exp)
                        })
                        .collect();
                    Map { data, ..a.clone() }
                }
                "Concat" => {
                    ensure!(node.int("axis")? == 3, "a Concat over channels");
                    let (a, b) = (
                        &maps[node.inputs[0].as_str()],
                        &maps[node.inputs[1].as_str()],
                    );
                    let (ea, eb) = (
                        self.exponent(&node.inputs[0])?,
                        self.exponent(&node.inputs[1])?,
                    );
                    let c = a.c + b.c;
                    let mut data = Vec::with_capacity(a.h * a.w * c);
                    for p in 0..a.h * a.w {
                        for &v in &a.data[p * a.c..(p + 1) * a.c] {
                            data.push(requantize(i64::from(v), ea - out_exp));
                        }
                        for &v in &b.data[p * b.c..(p + 1) * b.c] {
                            data.push(requantize(i64::from(v), eb - out_exp));
                        }
                    }
                    Map {
                        c,
                        data,
                        ..a.clone()
                    }
                }
                other => bail!("operator {other} is not supported"),
            };
            maps.insert(out, result);
        }
        Ok(maps
            .remove(self.output.as_str())
            .context("the output was never computed")?
            .data)
    }

    /// One convolution, as ESP-DL computes it.
    fn conv(&self, node: &Node, x: &Map, out_exp: i32) -> Result<Map> {
        let weight = self.constant(&node.inputs[1])?;
        let bias = self.constant(&node.inputs[2])?;
        let in_exp = self.exponent(&node.inputs[0])?;
        ensure!(
            bias.exponent == in_exp + weight.exponent,
            "a bias in product units"
        );
        let [kh, kw, ci, co] = weight.dims[..] else {
            bail!("weights of rank {}", weight.dims.len())
        };
        let group = node.int("group")? as usize;
        let stride = node.ints("strides")?[0] as usize;
        let pad = node.ints("pads")?[0] as usize;
        let depthwise = group > 1;
        let channels = if depthwise { ci } else { co };
        let filters = unpack(weight, group)?;
        let biases = bias.i32s();
        let (oh, ow) = (
            (x.h + 2 * pad - kh) / stride + 1,
            (x.w + 2 * pad - kw) / stride + 1,
        );
        let shift = in_exp + weight.exponent - out_exp;
        let mut data = vec![0i8; oh * ow * channels];
        for oy in 0..oh {
            for ox in 0..ow {
                for o in 0..channels {
                    let mut sum = i64::from(biases[o]);
                    for ky in 0..kh {
                        let Some(iy) = (oy * stride + ky).checked_sub(pad).filter(|&y| y < x.h)
                        else {
                            continue;
                        };
                        for kx in 0..kw {
                            let Some(ix) = (ox * stride + kx).checked_sub(pad).filter(|&v| v < x.w)
                            else {
                                continue;
                            };
                            let pixel = &x.data[(iy * x.w + ix) * x.c..];
                            if depthwise {
                                sum += i64::from(pixel[o])
                                    * i64::from(filters[(o * kh + ky) * kw + kx]);
                            } else {
                                let w = &filters[((o * kh + ky) * kw + kx) * ci..][..ci];
                                sum += pixel[..ci]
                                    .iter()
                                    .zip(w)
                                    .map(|(&a, &b)| i64::from(a) * i64::from(b))
                                    .sum::<i64>();
                            }
                        }
                    }
                    data[(oy * ow + ox) * channels + o] = requantize(sum, shift);
                }
            }
        }
        Ok(Map {
            h: oh,
            w: ow,
            c: channels,
            data,
        })
    }
}

/// An activation of the interpreter, channels-last.
#[derive(Clone)]
struct Map {
    h: usize,
    w: usize,
    c: usize,
    data: Vec<i8>,
}

/// `value * 2^shift`, rounded half up, saturated to `i8`.
fn requantize(value: i64, shift: i32) -> i8 {
    let q = if shift >= 0 {
        value << shift
    } else {
        let s = -shift;
        (value + (1 << (s - 1))) >> s
    };
    q.clamp(-128, 127) as i8
}

/// A convolution's weights from ESP-DL's `N16HWC16` order to
/// `[co][kh][kw][ci]` (or `[c][kh][kw]` for a depthwise one).
fn unpack(weight: &Constant, group: usize) -> Result<Vec<i8>> {
    ensure!(
        weight.layout.ends_with("N16HWC16"),
        "weights in {:?}",
        weight.layout
    );
    let [kh, kw, ci, co] = weight.dims[..] else {
        bail!("weights of rank {}", weight.dims.len())
    };
    let raw = weight.i8s();
    let mut out = vec![0i8; raw.len()];
    if group == 1 {
        for o in 0..co {
            for y in 0..kh {
                for x in 0..kw {
                    for c in 0..ci {
                        let from = (((o / 16) * kh + y) * kw + x) * ci * 16 + c * 16 + o % 16;
                        out[((o * kh + y) * kw + x) * ci + c] = raw[from];
                    }
                }
            }
        }
    } else {
        ensure!(co == 1 && group == ci, "a depthwise convolution");
        for c in 0..ci {
            for y in 0..kh {
                for x in 0..kw {
                    out[(c * kh + y) * kw + x] = raw[(((c / 16) * kh + y) * kw + x) * 16 + c % 16];
                }
            }
        }
    }
    Ok(out)
}

/// MFN_S8_V1's convolutions after the three pairs of halves are merged.
pub const LAYERS: usize = 50;

/// One layer of the import: a convolution, maybe merged with its twin,
/// with the PReLU that follows.
struct Layer {
    /// The FKB1 name.
    name: String,
    /// The input tensor.
    input: String,
    /// The weights, `[groups][taps][16]`.
    weights: Vec<u8>,
    /// Taps per output channel (after padding).
    taps: usize,
    /// Biases.
    bias: Vec<i32>,
    /// Per group: the shift.
    shifts: Vec<i32>,
    /// The PReLU: slopes, and per group the positive and negative shifts.
    prelu: Option<(Vec<i8>, Vec<[i32; 2]>)>,
}

/// The node producing each tensor.
fn producers(graph: &Graph) -> HashMap<&str, usize> {
    graph
        .nodes
        .iter()
        .enumerate()
        .flat_map(|(i, node)| node.outputs.iter().map(move |out| (out.as_str(), i)))
        .collect()
}

/// The consumers of each tensor.
fn consumers(graph: &Graph) -> HashMap<&str, Vec<usize>> {
    let mut map: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        for input in &node.inputs {
            map.entry(input.as_str()).or_default().push(i);
        }
    }
    map
}

/// The import of one convolution node (and its PReLU, if the node's only
/// consumer is one): its layer, and the tensor that ends it.
fn convolution(graph: &Graph, index: usize) -> Result<(Layer, String)> {
    let node = &graph.nodes[index];
    let weight = graph.constant(&node.inputs[1])?;
    let bias = graph.constant(&node.inputs[2])?;
    let in_exp = graph.exponent(&node.inputs[0])?;
    let conv_out = &node.outputs[0];
    let conv_exp = graph.exponent(conv_out)?;
    let [kh, kw, ci, co] = weight.dims[..] else {
        bail!("weights of rank {}", weight.dims.len())
    };
    let group = node.int("group")? as usize;
    let channels = if group > 1 { ci } else { co };
    ensure!(channels % 16 == 0, "{channels} channels");
    ensure!(
        node.attributes
            .get("activation")
            .is_none_or(|a| matches!(a, Attribute::Text(t) if t == "Linear")),
        "a fused activation"
    );
    let shift = in_exp + weight.exponent - conv_exp;
    ensure!(shift <= 0, "a left shift of {}", -shift);
    let groups = channels / 16;
    let per_tap = if group > 1 { 1 } else { ci };
    let taps = kh * kw * per_tap;
    // A full convolution runs as a 1x1 over its taps, whose count the
    // 1x1 kernel needs in multiples of sixteen: the stem's 27 become 32.
    // A depthwise one keeps its 9 (or 49).
    let padded = if group > 1 {
        taps
    } else {
        taps.next_multiple_of(16)
    };
    let mut weights = vec![0u8; groups * padded * 16];
    for g in 0..groups {
        let from = &weight.raw[g * taps * 16..(g + 1) * taps * 16];
        weights[g * padded * 16..g * padded * 16 + taps * 16].copy_from_slice(from);
    }
    let mut layer = Layer {
        name: node.inputs[1].trim_end_matches(".weight").to_string(),
        input: node.inputs[0].clone(),
        weights,
        taps: padded,
        bias: bias.i32s(),
        shifts: vec![-shift; groups],
        prelu: None,
    };
    let mut end = conv_out.clone();
    let users = consumers(graph);

    if let [user] = users
        .get(conv_out.as_str())
        .map(Vec::as_slice)
        .unwrap_or(&[])
        && graph.nodes[*user].op == "PRelu"
    {
        let prelu = &graph.nodes[*user];
        let alpha = graph.constant(&prelu.inputs[1])?;
        let out_exp = graph.exponent(&prelu.outputs[0])?;
        let positive = conv_exp - out_exp;
        let negative = out_exp - (conv_exp + alpha.exponent);
        ensure!(
            (0..=1).contains(&positive),
            "a PReLU positive shift of {positive}"
        );
        ensure!(negative >= 0, "a PReLU left shift of {}", -negative);
        layer.prelu = Some((alpha.i8s(), vec![[positive, negative]; groups]));
        end = prelu.outputs[0].clone();
    }
    Ok((layer, end))
}

/// Read MFN_S8_V1's `.espdl` file and write the FKB1 file `nn::mfn`
/// reads, after checking the graph is the one it implements.
///
/// # Errors
///
/// When the file cannot be read or written, or the graph breaks an
/// assumption of the kernels.
pub fn import(model: &Path, out: &Path) -> Result<()> {
    let graph = Graph::read(model)?;
    let producer = producers(&graph);
    // Every convolution, with the tensor that ends it (after its PReLU).
    let mut layers: Vec<(Layer, String)> = Vec::new();
    let mut merged_into: HashMap<String, usize> = HashMap::new();
    let mut residual: Vec<String> = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        match node.op.as_str() {
            "Conv" => layers.push(convolution(&graph, index)?),
            "PRelu" => {
                let source = &graph.nodes[producer[node.inputs[0].as_str()]];
                ensure!(source.op == "Conv", "a PReLU after {}", source.op);
            }
            "Concat" => {
                // Two halves: the first takes the second's groups.
                let find = |tensor: &str| layers.iter().position(|(_, end)| end == tensor);
                let a = find(&node.inputs[0]).context("a Concat of something else")?;
                let b = find(&node.inputs[1]).context("a Concat of something else")?;
                ensure!(b == a + 1, "a Concat of layers not side by side");
                let out_exp = graph.exponent(&node.outputs[0])?;
                for input in &node.inputs {
                    ensure!(graph.exponent(input)? == out_exp, "a Concat that rescales");
                }
                let (second, _) = layers.remove(b);
                let (first, end) = &mut layers[a];
                ensure!(first.input == second.input, "halves of different inputs");
                ensure!(
                    first.prelu.is_some() == second.prelu.is_some(),
                    "halves with and without a PReLU"
                );
                first.weights.extend(second.weights);
                first.bias.extend(second.bias);
                first.shifts.extend(second.shifts);
                if let (Some((alpha, shifts)), Some((alpha2, shifts2))) =
                    (&mut first.prelu, second.prelu)
                {
                    alpha.extend(alpha2);
                    shifts.extend(shifts2);
                }
                first.name = first.name.trim_end_matches("_1").to_string();
                merged_into.insert(second.name, a);
                *end = node.outputs[0].clone();
            }
            "Add" => {
                let out_exp = graph.exponent(&node.outputs[0])?;
                for input in &node.inputs {
                    ensure!(graph.exponent(input)? == out_exp, "an Add that rescales");
                }
                let branch = layers
                    .iter()
                    .find(|(_, end)| *end == node.inputs[1])
                    .context("an Add of something else")?;
                residual.push(branch.0.name.clone());
            }
            other => bail!("operator {other} is not supported"),
        }
    }
    ensure!(
        layers.len() == LAYERS,
        "{} layers, expected {LAYERS} (is this MFN_S8_V1?)",
        layers.len()
    );
    for name in &residual {
        ensure!(
            name.starts_with("res_") && name.ends_with("_conv_proj"),
            "a residual at {name}"
        );
    }
    ensure!(
        residual.len() == 12,
        "{} residual adds, expected 12",
        residual.len()
    );

    let mut writer = Writer::new();
    let input_exponent = graph.input_exponent()?;
    writer.add(BlobTensor {
        name: "input.exponent".into(),
        layout: "X".into(),
        data_type: DataType::I32,
        dims: vec![1],
        scale: 1.0,
        zero_point: 0,
        data: input_exponent.to_le_bytes().to_vec(),
    })?;
    for (layer, _) in &layers {
        let channels = layer.bias.len();
        let groups = channels / 16;
        let i32s = |values: &[i32]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        writer.add(BlobTensor {
            name: format!("{}.weight", layer.name),
            layout: "N16HWC16".into(),
            data_type: DataType::I8,
            dims: vec![groups, layer.taps, 16],
            scale: 1.0,
            zero_point: 0,
            data: layer.weights.clone(),
        })?;
        writer.add(BlobTensor {
            name: format!("{}.bias", layer.name),
            layout: "O".into(),
            data_type: DataType::I32,
            dims: vec![channels],
            scale: 1.0,
            zero_point: 0,
            data: i32s(&layer.bias),
        })?;
        writer.add(BlobTensor {
            name: format!("{}.shift", layer.name),
            layout: "G".into(),
            data_type: DataType::I32,
            dims: vec![groups],
            scale: 1.0,
            zero_point: 0,
            data: i32s(&layer.shifts),
        })?;
        if let Some((alpha, shifts)) = &layer.prelu {
            writer.add(BlobTensor {
                name: format!("{}.alpha", layer.name),
                layout: "O".into(),
                data_type: DataType::I8,
                dims: vec![channels],
                scale: 1.0,
                zero_point: 0,
                data: alpha.iter().map(|&v| v as u8).collect(),
            })?;
            let flat: Vec<i32> = shifts.iter().flatten().copied().collect();
            writer.add(BlobTensor {
                name: format!("{}.prelu", layer.name),
                layout: "GS".into(),
                data_type: DataType::I32,
                dims: vec![groups, 2],
                scale: 1.0,
                zero_point: 0,
                data: i32s(&flat),
            })?;
        }
    }
    let manifest = writer.manifest();
    fs::write(out, writer.finish()).with_context(|| format!("writing {}", out.display()))?;
    fs::write(out.with_extension("txt"), manifest)?;
    println!(
        "{} layers ({} merged from halves), {residual_count} residual adds, input exponent {input_exponent}: wrote {}",
        layers.len(),
        merged_into.len(),
        out.display(),
        residual_count = residual.len(),
    );
    for (layer, _) in &layers {
        println!(
            "  {:<28} {:>4} channels {:>3} taps, shifts {:?}{}",
            layer.name,
            layer.bias.len(),
            layer.taps,
            dedup(&layer.shifts),
            layer
                .prelu
                .as_ref()
                .map(|(_, s)| format!(
                    ", PReLU {:?}",
                    dedup(&s.iter().map(|p| p[0] * 100 + p[1]).collect::<Vec<_>>())
                ))
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// `values` without repeats next to each other.
fn dedup(values: &[i32]) -> Vec<i32> {
    let mut out = values.to_vec();
    out.dedup();
    out
}

/// The input of the graph for an aligned 112x112 RGB crop:
/// `round((byte - 127.5) / 127.5 * 2^-exponent)`, computed in `f64`
/// here and independently of `nn::mfn::input_value`.
pub fn quantize_input(rgb: &[u8], exponent: i32) -> Vec<i8> {
    let scale = f64::from(2f32.powi(-exponent));
    rgb.iter()
        .map(|&byte| {
            ((f64::from(byte) - 127.5) / 127.5 * scale)
                .round()
                .clamp(-128.0, 127.0) as i8
        })
        .collect()
}

/// Write the golden file of `model` for the aligned 112x112 crop `image`:
/// `input` (the quantized crop) and `embedding` (the graph's output),
/// both `i8`, from the interpreter.
///
/// # Errors
///
/// When a file cannot be read or written, the image is not 112x112, or
/// the graph does not run.
pub fn golden(model: &Path, image: &Path, out: &Path) -> Result<()> {
    let graph = Graph::read(model)?;
    let photo = image::open(image)
        .with_context(|| format!("reading {}", image.display()))?
        .to_rgb8();
    ensure!(
        (photo.width(), photo.height()) == (112, 112),
        "the crop is {}x{}, not 112x112",
        photo.width(),
        photo.height()
    );
    let input = quantize_input(photo.as_raw(), graph.input_exponent()?);
    let embedding = graph.run(&input)?;
    ensure!(embedding.len() == 512, "{} output values", embedding.len());
    let mut writer = Writer::new();
    for (name, values, dims, layout) in [
        ("input", &input, vec![112, 112, 3], "HWC"),
        ("embedding", &embedding, vec![512], "C"),
    ] {
        writer.add(BlobTensor {
            name: name.into(),
            layout: layout.into(),
            data_type: DataType::I8,
            dims,
            scale: 1.0,
            zero_point: 0,
            data: values.iter().map(|&v| v as u8).collect(),
        })?;
    }
    let manifest = writer.manifest();
    fs::write(out, writer.finish()).with_context(|| format!("writing {}", out.display()))?;
    fs::write(out.with_extension("txt"), manifest)?;
    println!(
        "embedding of {}: first values {:?}; wrote {}",
        image.display(),
        &embedding[..8],
        out.display()
    );
    Ok(())
}
