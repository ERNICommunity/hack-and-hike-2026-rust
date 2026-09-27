//! The "FKB1" file format: named tensors in one flat byte string.
//!
//! The developer tool `facekit` writes these files: the weights of a model,
//! and the reference inputs and outputs (golden vectors) that the tests
//! compare against. The firmware includes a weights file with
//! `include_bytes!` and reads it in place, so the format is made for
//! reading from a `&[u8]` without copying, allocating or unsafe code:
//!
//! ```text
//! offset  size  content
//! 0       4     magic "FKB1"
//! 4       4     number of entries, little-endian u32
//! 8       8     reserved, zero
//! 16      128   entry 0 (see below)
//! ...     128   entry N-1
//! ...           data of every tensor, each starting at a multiple of 16
//! ```
//!
//! One entry of the table of contents:
//!
//! ```text
//! offset  size  content
//! 0       64    name, UTF-8, padded with zeros
//! 64      1     data type: 0 = f32, 1 = i8, 2 = u8, 3 = i32
//! 65      1     rank: the number of dimensions, at most 4
//! 66      2     zero
//! 68      16    four dimensions as u32; unused ones are 1
//! 84      4     offset of the data from the start of the file, u32
//! 88      4     length of the data in bytes, u32
//! 92      4     scale, f32: for i8 tensors, real = scale * (value - zero)
//! 96      4     zero point, i32, for i8 tensors
//! 100     16    layout: what the dimensions mean, for example "OHWI"
//! 116     12    zero
//! ```
//!
//! All numbers are little-endian, as on the ESP32-S3 and on every
//! developer machine we use. The names are the PyTorch parameter names of
//! the model without the `model.` prefix, for example `stem.0.weight`. The
//! layout string tells how facekit arranged the dimensions for the
//! firmware, for example `OHWI` for a convolution weight stored as
//! `[out channels][kernel rows][kernel columns][in channels]`.
//!
//! Tensor data is read through iterators or copied into a caller's buffer.
//! `f32` values cannot be borrowed from bytes without `unsafe`, and the
//! firmware runs its networks on `i8` weights, which are plain bytes.

/// Include an FKB1 file from flash, aligned so that its `f32` tensors
/// can be read without copying.
///
/// `include_bytes!` gives a byte string of alignment 1, and an `f32`
/// needs 4. This macro puts the bytes inside a type that asks for 16, so
/// that every tensor's data (which the writer places at a multiple of
/// 16) is aligned as well. The file stays in flash and costs no RAM.
///
/// ```ignore
/// static WEIGHTS: &[u8] = include_fkb!("../../assets/models/yunet.int8.fkb");
/// ```
///
/// The path is relative to the file that uses the macro, as with
/// `include_bytes!`.
#[macro_export]
macro_rules! include_fkb {
    ($path:literal) => {{
        /// A wrapper that raises the alignment of the bytes it holds.
        #[repr(C)]
        struct Aligned<T: ?Sized> {
            /// Empty, but its type asks for 16-byte alignment.
            _align: [u128; 0],
            /// The file.
            bytes: T,
        }
        static ALIGNED: &Aligned<[u8]> = &Aligned {
            _align: [],
            bytes: *include_bytes!($path),
        };
        &ALIGNED.bytes
    }};
}

/// The first four bytes of every file.
pub const MAGIC: [u8; 4] = *b"FKB1";
/// Bytes before the table of contents.
pub const HEADER_LEN: usize = 16;
/// Bytes of one table entry.
pub const ENTRY_LEN: usize = 128;
/// Bytes reserved for a tensor's name.
pub const NAME_LEN: usize = 64;
/// Bytes reserved for a tensor's layout string.
pub const LAYOUT_LEN: usize = 16;
/// Every tensor's data starts at a multiple of this.
pub const DATA_ALIGN: usize = 16;
/// The largest rank an entry can describe.
pub const MAX_RANK: usize = 4;

/// The element type of a tensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataType {
    /// 32-bit float.
    F32,
    /// Signed 8-bit integer, with a scale and zero point in the entry.
    I8,
    /// Unsigned byte, for images.
    U8,
    /// Signed 32-bit integer, for biases of the integer networks.
    I32,
}

impl DataType {
    /// The type for the code stored in a file.
    fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::F32),
            1 => Some(Self::I8),
            2 => Some(Self::U8),
            3 => Some(Self::I32),
            _ => None,
        }
    }

    /// The code stored in a file.
    pub fn code(self) -> u8 {
        match self {
            Self::F32 => 0,
            Self::I8 => 1,
            Self::U8 => 2,
            Self::I32 => 3,
        }
    }

    /// Bytes per element.
    pub fn size(self) -> usize {
        match self {
            Self::F32 | Self::I32 => 4,
            Self::I8 | Self::U8 => 1,
        }
    }
}

/// Why a byte string is not a valid file. The index is the entry number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobError {
    /// Shorter than the header, or than the table of contents it announces.
    TooShort,
    /// The first four bytes are not [`MAGIC`].
    BadMagic,
    /// The name is empty or not UTF-8, or the layout is not UTF-8.
    BadName(usize),
    /// The data type code or the rank is unknown.
    BadType(usize),
    /// The data offset is not aligned, or the data does not fit in the
    /// file, or its length does not match the dimensions.
    BadRange(usize),
}

/// A parsed file. Cheap to copy: it borrows the bytes.
#[derive(Clone, Copy, Debug)]
pub struct Blob<'a> {
    /// The whole file.
    bytes: &'a [u8],
    /// The number of entries.
    count: usize,
}

/// One tensor of a file: its description and a borrow of its bytes.
#[derive(Clone, Copy, Debug)]
pub struct Entry<'a> {
    /// The tensor's name.
    pub name: &'a str,
    /// What the dimensions mean, for example `OHWI` or `HWC`.
    pub layout: &'a str,
    /// The element type.
    pub data_type: DataType,
    /// The dimensions; only the first `rank` are meaningful.
    dims: [usize; MAX_RANK],
    /// The number of dimensions.
    rank: usize,
    /// For `i8` tensors: real value = `scale * (value - zero_point)`.
    pub scale: f32,
    /// For `i8` tensors, see `scale`.
    pub zero_point: i32,
    /// The tensor's bytes, little-endian.
    bytes: &'a [u8],
    /// Where the bytes start in the file.
    offset: usize,
}

/// A little-endian u32 at `offset`. The caller checks the bounds.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// The string in a zero-padded field, or `None` when it is not UTF-8.
fn padded_str(field: &[u8]) -> Option<&str> {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    core::str::from_utf8(&field[..end]).ok()
}

impl<'a> Blob<'a> {
    /// Check the header and every entry of `bytes`.
    ///
    /// # Errors
    ///
    /// When `bytes` is not a valid file; see [`BlobError`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BlobError> {
        if bytes.len() < HEADER_LEN {
            return Err(BlobError::TooShort);
        }
        if bytes[..4] != MAGIC {
            return Err(BlobError::BadMagic);
        }
        let count = u32_at(bytes, 4) as usize;
        if bytes.len() < HEADER_LEN + count * ENTRY_LEN {
            return Err(BlobError::TooShort);
        }
        let blob = Self { bytes, count };
        for index in 0..count {
            blob.entry_checked(index)?;
        }
        Ok(blob)
    }

    /// The number of tensors.
    pub fn len(&self) -> usize {
        self.count
    }

    /// The whole file.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The bytes of the table-of-contents entry number `index`.
    ///
    /// # Panics
    ///
    /// When `index` is `len()` or more.
    pub fn entry_bytes(&self, index: usize) -> &'a [u8] {
        assert!(index < self.count, "entry {index} of {}", self.count);
        let start = HEADER_LEN + index * ENTRY_LEN;
        &self.bytes[start..start + ENTRY_LEN]
    }

    /// Whether the file has no tensors.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Entry number `index`.
    ///
    /// # Panics
    ///
    /// When `index` is `len()` or more.
    pub fn entry(&self, index: usize) -> Entry<'a> {
        assert!(index < self.count, "entry {index} of {}", self.count);
        // `parse` checked every entry, so this cannot fail.
        self.entry_checked(index)
            .unwrap_or_else(|error| panic!("entry {index} was valid at parse time: {error:?}"))
    }

    /// Every tensor, in file order.
    pub fn entries(&self) -> impl Iterator<Item = Entry<'a>> + '_ {
        (0..self.count).map(|index| self.entry(index))
    }

    /// The tensor called `name`, if there is one.
    pub fn get(&self, name: &str) -> Option<Entry<'a>> {
        self.entries().find(|entry| entry.name == name)
    }

    /// Decode entry `index`, checking every field.
    fn entry_checked(&self, index: usize) -> Result<Entry<'a>, BlobError> {
        let start = HEADER_LEN + index * ENTRY_LEN;
        let field = &self.bytes[start..start + ENTRY_LEN];

        let name = padded_str(&field[..NAME_LEN]).ok_or(BlobError::BadName(index))?;
        if name.is_empty() {
            return Err(BlobError::BadName(index));
        }
        let layout = padded_str(&field[100..100 + LAYOUT_LEN]).ok_or(BlobError::BadName(index))?;
        let data_type = DataType::from_code(field[64]).ok_or(BlobError::BadType(index))?;
        let rank = field[65] as usize;
        if rank > MAX_RANK {
            return Err(BlobError::BadType(index));
        }
        let mut dims = [1usize; MAX_RANK];
        for (axis, dim) in dims.iter_mut().enumerate() {
            *dim = u32_at(field, 68 + axis * 4) as usize;
        }
        let offset = u32_at(field, 84) as usize;
        let len = u32_at(field, 88) as usize;
        let scale = f32::from_le_bytes([field[92], field[93], field[94], field[95]]);
        let zero_point = i32::from_le_bytes([field[96], field[97], field[98], field[99]]);

        let elements: usize = dims[..rank]
            .iter()
            .try_fold(1usize, |product, &dim| product.checked_mul(dim))
            .ok_or(BlobError::BadRange(index))?;
        if !offset.is_multiple_of(DATA_ALIGN)
            || len != elements * data_type.size()
            || offset
                .checked_add(len)
                .is_none_or(|end| end > self.bytes.len())
        {
            return Err(BlobError::BadRange(index));
        }
        Ok(Entry {
            name,
            layout,
            data_type,
            dims,
            rank,
            scale,
            zero_point,
            bytes: &self.bytes[offset..offset + len],
            offset,
        })
    }
}

impl<'a> Entry<'a> {
    /// The dimensions, in the order the layout string names them.
    pub fn shape(&self) -> &[usize] {
        &self.dims[..self.rank]
    }

    /// The number of elements.
    pub fn element_count(&self) -> usize {
        self.shape().iter().product()
    }

    /// The raw little-endian bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Where the bytes start in the file.
    pub fn data_offset(&self) -> usize {
        self.offset
    }

    /// The values of an `f32` tensor, in order.
    ///
    /// # Panics
    ///
    /// When the tensor is not `f32`.
    pub fn f32s(&self) -> impl Iterator<Item = f32> + 'a {
        assert_eq!(self.data_type, DataType::F32, "{} is not f32", self.name);
        self.bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
    }

    /// Copy an `f32` tensor into `out`.
    ///
    /// # Panics
    ///
    /// When the tensor is not `f32` or `out` has the wrong length.
    pub fn read_f32s(&self, out: &mut [f32]) {
        assert_eq!(
            out.len(),
            self.element_count(),
            "{}: wrong buffer length",
            self.name
        );
        for (slot, value) in out.iter_mut().zip(self.f32s()) {
            *slot = value;
        }
    }

    /// The values of an `i32` tensor, in order.
    ///
    /// # Panics
    ///
    /// When the tensor is not `i32`.
    pub fn i32s(&self) -> impl Iterator<Item = i32> + 'a {
        assert_eq!(self.data_type, DataType::I32, "{} is not i32", self.name);
        self.bytes
            .chunks_exact(4)
            .map(|chunk| i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
    }

    /// The values of an `i8` tensor, in order.
    ///
    /// # Panics
    ///
    /// When the tensor is not `i8`.
    pub fn i8s(&self) -> impl Iterator<Item = i8> + 'a {
        assert_eq!(self.data_type, DataType::I8, "{} is not i8", self.name);
        self.bytes.iter().map(|&byte| byte as i8)
    }

    /// An `i8` tensor as a slice, without copying.
    ///
    /// # Panics
    ///
    /// When the tensor is not `i8`.
    pub fn i8_slice(&self) -> &'a [i8] {
        assert_eq!(self.data_type, DataType::I8, "{} is not i8", self.name);
        bytemuck::cast_slice(self.bytes)
    }

    /// An `f32` tensor as a slice, without copying. This needs the file to
    /// start at an address that is a multiple of 4; the firmware puts it
    /// in a 16-byte aligned static, and `Vec<u8>` on a computer is aligned
    /// enough in practice. Returns `None` when it is not.
    pub fn f32_slice(&self) -> Option<&'a [f32]> {
        assert_eq!(self.data_type, DataType::F32, "{} is not f32", self.name);
        bytemuck::try_cast_slice(self.bytes).ok()
    }
}
