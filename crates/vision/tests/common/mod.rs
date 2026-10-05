//! Helpers shared by the network tests: weights and golden vectors from
//! the FKB1 fixtures, layout conversion, and comparison with a tolerance.
#![allow(dead_code)]

use std::collections::HashMap;

use std::collections::HashSet;

use hack_and_hike_vision::{
    blob::{Blob, DataType, Entry},
    image::RgbImage,
    nn::{Shape, Weights, mfn, pack, s8::Plan},
};

/// Every `f32` and `i8` tensor of an FKB1 file, copied into memory, by
/// name.
pub struct Tensors {
    /// The `f32` tensors.
    by_name: HashMap<String, (Vec<usize>, Vec<f32>)>,
    /// The `i8` tensors.
    i8_by_name: HashMap<String, Vec<i8>>,
    /// The shapes of the `i8` tensors.
    i8_shapes: HashMap<String, Vec<usize>>,
    /// The `u8` tensors (images): shape and bytes.
    u8_by_name: HashMap<String, (Vec<usize>, Vec<u8>)>,
    /// The layout string of each tensor, for example `NCHW`.
    layouts: HashMap<String, String>,
    /// The `i8` weights grouped by eight output channels for the lane
    /// kernels.
    packed: HashSet<String>,
}

impl Tensors {
    /// Read every `f32` and `i8` tensor of `bytes`.
    pub fn load(bytes: &[u8]) -> Self {
        let blob = Blob::parse(bytes).expect("valid FKB1 file");
        let by_name = blob
            .entries()
            .filter(|entry| entry.data_type == DataType::F32)
            .map(|entry: Entry| {
                (
                    entry.name.to_string(),
                    (entry.shape().to_vec(), entry.f32s().collect()),
                )
            })
            .collect();
        let i8_by_name = blob
            .entries()
            .filter(|entry| entry.data_type == DataType::I8)
            .map(|entry: Entry| (entry.name.to_string(), entry.i8s().collect()))
            .collect();
        let i8_shapes = blob
            .entries()
            .filter(|entry| entry.data_type == DataType::I8)
            .map(|entry: Entry| (entry.name.to_string(), entry.shape().to_vec()))
            .collect();
        let u8_by_name = blob
            .entries()
            .filter(|entry| entry.data_type == DataType::U8)
            .map(|entry: Entry| {
                (
                    entry.name.to_string(),
                    (entry.shape().to_vec(), entry.bytes().to_vec()),
                )
            })
            .collect();
        let layouts = blob
            .entries()
            .map(|entry| (entry.name.to_string(), entry.layout.to_string()))
            .collect();
        Self {
            by_name,
            i8_by_name,
            i8_shapes,
            u8_by_name,
            layouts,
            packed: HashSet::new(),
        }
    }

    /// Group every linear and full-convolution `i8` weight by eight
    /// output channels, as the board does in its PSRAM copy.
    pub fn pack_for_lanes(&mut self) {
        let names: Vec<String> = self.i8_by_name.keys().cloned().collect();
        for name in names {
            let layout = self.layouts[&name].clone();
            let outputs = match self.i8_shapes.get(&name) {
                Some(shape) => shape[0],
                None => continue,
            };
            if !matches!(layout.as_str(), "OI" | "OHWI")
                || !outputs.is_multiple_of(pack::GROUP)
                || self.packed.contains(&name)
            {
                continue;
            }
            let data = self.i8_by_name.get_mut(&name).expect("i8 tensor");
            let per_output = data.len() / outputs;
            let rows: Vec<u8> = data.iter().map(|&w| w as u8).collect();
            let mut packed = vec![0u8; rows.len()];
            pack::pack_rows(&rows, outputs, per_output, &mut packed);
            *data = packed.iter().map(|&w| w as i8).collect();
            self.packed.insert(name);
        }
    }

    /// A `u8` tensor (an image): its shape and bytes.
    ///
    /// # Panics
    ///
    /// When there is no such tensor.
    pub fn u8_tensor(&self, name: &str) -> (&[usize], &[u8]) {
        let (shape, bytes) = self
            .u8_by_name
            .get(name)
            .unwrap_or_else(|| panic!("no u8 tensor called {name}"));
        (shape, bytes)
    }

    /// The names of the `f32` tensors, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// The layout string of `name`, for example `OHWI`.
    ///
    /// # Panics
    ///
    /// When there is no such tensor.
    pub fn layout(&self, name: &str) -> &str {
        self.layouts
            .get(name)
            .unwrap_or_else(|| panic!("no tensor called {name}"))
    }

    /// The shape and values of `name`.
    ///
    /// # Panics
    ///
    /// When there is no such tensor.
    pub fn tensor(&self, name: &str) -> (&[usize], &[f32]) {
        let (shape, values) = self
            .by_name
            .get(name)
            .unwrap_or_else(|| panic!("no tensor called {name}"));
        (shape, values)
    }

    /// A golden activation stored as `[1][C][H][W]`, converted to the
    /// channels-last layout of the kernels.
    pub fn activation_nchw(&self, name: &str) -> (Shape, Vec<f32>) {
        let (dims, values) = self.tensor(name);
        let [1, channels, height, width] = dims[..] else {
            panic!("{name} is not [1, C, H, W]: {dims:?}");
        };
        let shape = Shape::new(height, width, channels);
        let mut hwc = vec![0.0f32; shape.len()];
        for c in 0..channels {
            for y in 0..height {
                for x in 0..width {
                    hwc[shape.offset(x, y) + c] = values[(c * height + y) * width + x];
                }
            }
        }
        (shape, hwc)
    }

    /// A golden tensor stored as `[1][N][C]` (tokens by channels) or
    /// `[1][1][1][C]` or `[1][C]`, as a flat slice: already channels-last.
    pub fn flat(&self, name: &str) -> &[f32] {
        self.tensor(name).1
    }

    /// Whether the golden file has a tensor called `name`.
    pub fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// A golden activation in the kernels' channels-last order, whatever
    /// layout the file stored it in: `NCHW` tensors are converted, the
    /// others (`NAK`, `NHWC`, `NC`) are already in that order.
    pub fn activation(&self, name: &str) -> Vec<f32> {
        if self.layouts.get(name).map(String::as_str) == Some("NCHW") {
            self.activation_nchw(name).1
        } else {
            self.flat(name).to_vec()
        }
    }
}

impl Weights for Tensors {
    fn get(&self, name: &str) -> &[f32] {
        self.tensor(name).1
    }

    fn packed(&self, name: &str) -> bool {
        self.packed.contains(name)
    }

    fn get_i8(&self, name: &str) -> &[i8] {
        self.i8_by_name
            .get(name)
            .unwrap_or_else(|| panic!("no i8 tensor called {name}"))
    }
}

/// The largest absolute difference and the largest absolute golden value.
pub fn compare(actual: &[f32], golden: &[f32]) -> (f32, f32) {
    assert_eq!(actual.len(), golden.len(), "lengths differ");
    let mut max_error = 0.0f32;
    let mut max_value = 0.0f32;
    for (a, g) in actual.iter().zip(golden) {
        assert!(a.is_finite(), "non-finite value {a}");
        max_error = max_error.max((a - g).abs());
        max_value = max_value.max(g.abs());
    }
    (max_error, max_value)
}

/// Assert that `actual` matches `golden` within `atol + rtol * max|golden|`,
/// and print the numbers so a failing block is easy to read.
pub fn assert_close(name: &str, actual: &[f32], golden: &[f32], atol: f32, rtol: f32) {
    let (max_error, max_value) = compare(actual, golden);
    let allowed = atol + rtol * max_value;
    println!(
        "{name:<40} max error {max_error:.2e} (largest value {max_value:.3}, allowed {allowed:.2e})"
    );
    assert!(
        max_error <= allowed,
        "{name}: max error {max_error} exceeds {allowed}"
    );
}

/// The part of `buffer` from its first 16-byte boundary.
pub fn aligned<T>(buffer: &mut [T]) -> &mut [T] {
    let skip = buffer.as_ptr().align_offset(16);
    &mut buffer[skip..]
}

/// The recognizer's weights (MFN_S8_V1), as the firmware carries them.
pub const MFN_WEIGHTS: &[u8] = include_bytes!("../../../../assets/models/mfn_s8_v1.fkb");

/// The aligned fixture face, `face_112x112.jpg`, as RGB bytes.
pub fn fixture_face() -> Vec<u8> {
    image::load_from_memory(include_bytes!("../fixtures/face_112x112.jpg"))
        .expect("the fixture decodes")
        .to_rgb8()
        .into_raw()
}

/// The recognizer's raw embedding of an aligned 112x112 RGB crop, as the
/// board computes it, as numbers.
pub fn mfn_embedding(crop: &RgbImage<'_>) -> Vec<f32> {
    let blob = Blob::parse(MFN_WEIGHTS).expect("the recognizer's weights");
    let mut plans = vec![Plan::ZERO; mfn::MODEL_PLANS];
    let model = mfn::Model::compile(&blob, &mut plans);
    let mut input = vec![0i8; 112 * 112 * 3];
    mfn::input_i8(crop, &mut input);
    let (mut first, mut second) = (vec![0i8; mfn::TENSOR_LEN], vec![0i8; mfn::TENSOR_LEN]);
    let mut ring = vec![0i8; mfn::FAST_RING];
    let mut filtered = vec![0i8; mfn::FAST_FILTERED];
    let mut staging = vec![0i8; mfn::FAST_STAGING];
    let mut columns = vec![0i8; mfn::COLUMNS_LEN];
    let mut raw = vec![0i8; mfn::EMBEDDING_LEN];
    model.forward(
        &input,
        mfn::Scratch {
            tensors: [&mut first, &mut second],
            ring: &mut ring,
            filtered: &mut filtered,
            staging: &mut staging,
            columns: &mut columns,
        },
        &mut raw,
    );
    raw.iter().map(|&value| f32::from(value)).collect()
}
