//! Writing FKB1 files. The format and the reader live in
//! `hack_and_hike_vision::blob`; this is the other half, which only the
//! developer tool needs.

use anyhow::{Result, bail};
use hack_and_hike_vision::blob::{
    DATA_ALIGN, DataType, ENTRY_LEN, HEADER_LEN, LAYOUT_LEN, MAGIC, MAX_RANK, NAME_LEN,
};

/// One tensor to write. The convenience methods of [`Writer`] build it for
/// the common element types.
pub struct Tensor {
    /// The name, at most `NAME_LEN` bytes.
    pub name: String,
    /// The layout string, at most `LAYOUT_LEN` bytes.
    pub layout: String,
    /// The element type.
    pub data_type: DataType,
    /// The dimensions.
    pub dims: Vec<usize>,
    /// For `i8` tensors.
    pub scale: f32,
    /// For `i8` tensors.
    pub zero_point: i32,
    /// The little-endian bytes.
    pub data: Vec<u8>,
}

/// Collects tensors and writes them as one file.
#[derive(Default)]
pub struct Writer {
    /// The tensors, in the order they were added.
    entries: Vec<Tensor>,
}

impl Writer {
    /// An empty file.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tensor.
    ///
    /// # Errors
    ///
    /// When the name or layout is too long or empty, the rank is above
    /// `MAX_RANK`, the name was added before, or the byte length does not
    /// match the dimensions.
    pub fn add(&mut self, tensor: Tensor) -> Result<()> {
        let Tensor {
            name,
            layout,
            data_type,
            dims,
            data,
            ..
        } = &tensor;
        if name.is_empty() || name.len() > NAME_LEN {
            bail!("tensor name {name:?} is empty or longer than {NAME_LEN} bytes");
        }
        if layout.len() > LAYOUT_LEN {
            bail!("{name}: layout {layout:?} is longer than {LAYOUT_LEN} bytes");
        }
        if dims.len() > MAX_RANK {
            bail!("{name}: rank {} is above {MAX_RANK}", dims.len());
        }
        if self.entries.iter().any(|entry| entry.name == *name) {
            bail!("tensor {name} was added twice");
        }
        let elements: usize = dims.iter().product();
        if data.len() != elements * data_type.size() {
            bail!(
                "{name}: {} bytes do not match dimensions {dims:?} of {data_type:?}",
                data.len()
            );
        }
        self.entries.push(tensor);
        Ok(())
    }

    /// Add an `f32` tensor.
    ///
    /// # Errors
    ///
    /// See [`Writer::add`].
    pub fn add_f32(
        &mut self,
        name: &str,
        layout: &str,
        dims: &[usize],
        values: &[f32],
    ) -> Result<()> {
        self.add(Tensor {
            name: name.to_string(),
            layout: layout.to_string(),
            data_type: DataType::F32,
            dims: dims.to_vec(),
            scale: 0.0,
            zero_point: 0,
            data: values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect(),
        })
    }

    /// Add a `u8` tensor, for example an image.
    ///
    /// # Errors
    ///
    /// See [`Writer::add`].
    pub fn add_u8(
        &mut self,
        name: &str,
        layout: &str,
        dims: &[usize],
        values: &[u8],
    ) -> Result<()> {
        self.add(Tensor {
            name: name.to_string(),
            layout: layout.to_string(),
            data_type: DataType::U8,
            dims: dims.to_vec(),
            scale: 0.0,
            zero_point: 0,
            data: values.to_vec(),
        })
    }

    /// The bytes of the file.
    pub fn finish(self) -> Vec<u8> {
        let table_end = HEADER_LEN + self.entries.len() * ENTRY_LEN;
        let mut file = Vec::new();
        file.extend_from_slice(&MAGIC);
        file.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        file.extend_from_slice(&[0u8; 8]);

        // Lay out the data first, so the table can point at it.
        let mut offset = table_end.next_multiple_of(DATA_ALIGN);
        let mut offsets = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            offsets.push(offset);
            offset = (offset + entry.data.len()).next_multiple_of(DATA_ALIGN);
        }

        for (entry, &data_offset) in self.entries.iter().zip(&offsets) {
            let mut field = [0u8; ENTRY_LEN];
            field[..entry.name.len()].copy_from_slice(entry.name.as_bytes());
            field[64] = entry.data_type.code();
            field[65] = entry.dims.len() as u8;
            for axis in 0..MAX_RANK {
                let dim = entry.dims.get(axis).copied().unwrap_or(1) as u32;
                field[68 + axis * 4..72 + axis * 4].copy_from_slice(&dim.to_le_bytes());
            }
            field[84..88].copy_from_slice(&(data_offset as u32).to_le_bytes());
            field[88..92].copy_from_slice(&(entry.data.len() as u32).to_le_bytes());
            field[92..96].copy_from_slice(&entry.scale.to_le_bytes());
            field[96..100].copy_from_slice(&entry.zero_point.to_le_bytes());
            field[100..100 + entry.layout.len()].copy_from_slice(entry.layout.as_bytes());
            file.extend_from_slice(&field);
        }

        for (entry, &data_offset) in self.entries.iter().zip(&offsets) {
            file.resize(data_offset, 0);
            file.extend_from_slice(&entry.data);
        }
        file.resize(offset, 0);
        file
    }

    /// A human-readable listing: one line per tensor.
    pub fn manifest(&self) -> String {
        let mut text = String::new();
        let mut total = 0usize;
        for entry in &self.entries {
            total += entry.data.len();
            text.push_str(&format!(
                "{:<56} {:<5} {:<6} {:?}\n",
                entry.name,
                format!("{:?}", entry.data_type).to_lowercase(),
                entry.layout,
                entry.dims
            ));
        }
        text.push_str(&format!(
            "{} tensors, {} bytes of data\n",
            self.entries.len(),
            total
        ));
        text
    }
}
