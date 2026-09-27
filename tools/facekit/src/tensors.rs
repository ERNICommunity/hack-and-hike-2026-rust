//! An FKB1 file in memory, usable as the networks' `Weights`.

use std::{collections::HashMap, fs, path::Path};

use std::collections::HashSet;

use anyhow::{Context, Result};
use hack_and_hike_vision::{
    blob::{Blob, DataType},
    nn::{Weights, pack},
};

/// One tensor of a file.
pub struct Tensor {
    /// What the dimensions mean.
    pub layout: String,
    /// The dimensions.
    pub shape: Vec<usize>,
    /// The values, when the tensor is `f32`.
    pub f32s: Vec<f32>,
    /// The values, when the tensor is `i8`.
    pub i8s: Vec<i8>,
}

/// Every tensor of an FKB1 file, by name, in file order.
pub struct Tensors {
    /// The tensors.
    pub by_name: HashMap<String, Tensor>,
    /// The names, in file order.
    pub names: Vec<String>,
    /// The `i8` weights grouped by eight output channels for the lane
    /// kernels (see [`pack_for_lanes`](Self::pack_for_lanes)).
    packed: HashSet<String>,
}

impl Tensors {
    /// Read `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or parsed.
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let blob = Blob::parse(&bytes)
            .map_err(|error| anyhow::anyhow!("{}: {error:?}", path.display()))?;
        let mut by_name = HashMap::new();
        let mut names = Vec::new();
        for entry in blob.entries() {
            let (f32s, i8s) = match entry.data_type {
                DataType::F32 => (entry.f32s().collect(), Vec::new()),
                DataType::I8 => (Vec::new(), entry.i8s().collect()),
                DataType::U8 | DataType::I32 => continue,
            };
            names.push(entry.name.to_string());
            by_name.insert(
                entry.name.to_string(),
                Tensor {
                    layout: entry.layout.to_string(),
                    shape: entry.shape().to_vec(),
                    f32s,
                    i8s,
                },
            );
        }
        Ok(Self {
            by_name,
            names,
            packed: HashSet::new(),
        })
    }

    /// Group every linear and full-convolution `i8` weight by eight
    /// output channels, as the board does in its PSRAM copy: what the
    /// integer networks need.
    pub fn pack_for_lanes(&mut self) {
        for name in &self.names {
            let tensor = self.by_name.get_mut(name).expect("named tensor");
            let packable = !tensor.i8s.is_empty()
                && matches!(tensor.layout.as_str(), "OI" | "OHWI")
                && tensor.shape[0].is_multiple_of(pack::GROUP)
                && !self.packed.contains(name);
            if !packable {
                continue;
            }
            let outputs = tensor.shape[0];
            let per_output = tensor.i8s.len() / outputs;
            let rows: Vec<u8> = tensor.i8s.iter().map(|&w| w as u8).collect();
            let mut packed = vec![0u8; rows.len()];
            pack::pack_rows(&rows, outputs, per_output, &mut packed);
            tensor.i8s = packed.iter().map(|&w| w as i8).collect();
            self.packed.insert(name.clone());
        }
    }

    /// The tensor called `name`.
    ///
    /// # Panics
    ///
    /// When there is none.
    pub fn tensor(&self, name: &str) -> &Tensor {
        self.by_name
            .get(name)
            .unwrap_or_else(|| panic!("no tensor called {name}"))
    }
}

impl Weights for Tensors {
    fn get(&self, name: &str) -> &[f32] {
        &self.tensor(name).f32s
    }

    fn get_i8(&self, name: &str) -> &[i8] {
        &self.tensor(name).i8s
    }

    fn packed(&self, name: &str) -> bool {
        self.packed.contains(name)
    }
}
