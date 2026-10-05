//! The recognizer as the board runs it: MFN_S8_V1 through `nn::mfn`, on
//! the same integer arithmetic, with the `.espdl` interpreter as an
//! optional reference beside it.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use hack_and_hike_vision::{
    blob::Blob,
    image::RgbImage,
    nn::{
        mfn::{self, Model, Scratch},
        s8::Plan,
    },
};

use crate::{embed::normalize, espdl::Graph};

/// MFN_S8_V1 compiled once, shared by every thread, with the reference
/// if one was given.
pub struct Recognizer {
    /// The model, over a weights file and plans kept for the program's
    /// life.
    model: Model<'static>,
    /// The `.espdl` interpreter, if wanted.
    reference: Option<Graph>,
}

impl Recognizer {
    /// Load the FKB1 file `weights` (from `import-espdl`) and, when
    /// given, the `.espdl` file `reference`.
    ///
    /// # Errors
    ///
    /// When a file cannot be read or parsed.
    pub fn load(weights: &Path, reference: Option<&Path>) -> Result<Self> {
        let bytes: &'static [u8] = fs::read(weights)
            .with_context(|| format!("reading {}", weights.display()))?
            .leak();
        let blob = Blob::parse(bytes)
            .map_err(|error| anyhow::anyhow!("{}: {error:?}", weights.display()))?;
        let plans = vec![Plan::ZERO; mfn::MODEL_PLANS].leak();
        let model = Model::compile(&blob, plans);
        let reference = reference.map(Graph::read).transpose()?;
        Ok(Self { model, reference })
    }

    /// Whether a reference was loaded.
    pub fn has_reference(&self) -> bool {
        self.reference.is_some()
    }

    /// The L2-normalized embedding of an aligned crop, as the board
    /// computes it.
    pub fn embed(&self, crop: &RgbImage<'_>, buffers: &mut Buffers) -> Vec<f32> {
        mfn::input_i8(crop, &mut buffers.input);
        let mut raw = [0i8; mfn::EMBEDDING_LEN];
        self.model.forward(
            &buffers.input,
            Scratch {
                tensors: [&mut buffers.first, &mut buffers.second],
                ring: &mut buffers.ring,
                filtered: &mut buffers.filtered,
                staging: &mut buffers.staging,
                columns: &mut buffers.columns,
            },
            &mut raw,
        );
        let mut values: Vec<f32> = raw.iter().map(|&v| f32::from(v)).collect();
        normalize(&mut values);
        values
    }

    /// The L2-normalized embedding of an aligned crop from the
    /// interpreter, when a reference was loaded and it runs.
    pub fn reference(&self, crop: &RgbImage<'_>) -> Option<Vec<f32>> {
        let graph = self.reference.as_ref()?;
        let input = crate::espdl::quantize_input(crop.data(), mfn::INPUT_EXPONENT);
        let raw = graph.run(&input).ok()?;
        let mut values: Vec<f32> = raw.iter().map(|&v| f32::from(v)).collect();
        normalize(&mut values);
        Some(values)
    }
}

/// One thread's working memory for [`Recognizer::embed`].
pub struct Buffers {
    /// The quantized crop.
    input: Vec<i8>,
    /// The two tensors between blocks.
    first: Vec<i8>,
    /// See `first`.
    second: Vec<i8>,
    /// The ring of wide rows, large enough for bands of seven rows.
    ring: Vec<i8>,
    /// A band of depthwise rows.
    filtered: Vec<i8>,
    /// A band of output rows.
    staging: Vec<i8>,
    /// The stem's columns.
    columns: Vec<i8>,
}

impl Default for Buffers {
    fn default() -> Self {
        Self {
            input: vec![0; 112 * 112 * 3],
            first: vec![0; mfn::TENSOR_LEN],
            second: vec![0; mfn::TENSOR_LEN],
            ring: vec![0; mfn::FAST_RING.max(mfn::MIN_RING)],
            filtered: vec![0; mfn::FAST_FILTERED],
            staging: vec![0; mfn::FAST_STAGING],
            columns: vec![0; mfn::COLUMNS_LEN],
        }
    }
}
