//! Weights arranged for the vector unit: a copy of a weights file in
//! which every linear and full-convolution weight is grouped by eight
//! output channels.
//!
//! The vector unit's multiply-accumulate keeps eight 40-bit sums in its
//! lanes. With one input value broadcast to every lane and the eight
//! weights of eight filters at that input in the other operand, one
//! instruction advances eight outputs by one product, and a whole row of
//! outputs comes out of one loop with no per-output work. That needs the
//! weights of a group of eight filters interleaved: `[group][input][8]`
//! instead of `[output][input]`. The file keeps the plain layout, which
//! the computer's scalar loops read directly; the board builds this copy
//! in PSRAM when it starts, which also puts the weights on the faster
//! memory.
//!
//! The copy is a valid FKB1 file of the same size: every entry keeps its
//! name, type, shape and place, and a packed tensor's layout string gets
//! the suffix [`PACKED_SUFFIX`] (`OI/8`, `OHWI/8`). [`BlobWeights`] reports
//! the suffix through [`Weights::packed`], and the kernels index the
//! weights accordingly.

use crate::blob::{Blob, DataType, ENTRY_LEN, Entry, HEADER_LEN, LAYOUT_LEN};

/// Output channels in one group.
pub const GROUP: usize = 8;
/// What a packed tensor's layout string ends with.
pub const PACKED_SUFFIX: &str = "/8";
/// Where the layout string starts in a table-of-contents entry.
const LAYOUT_OFFSET: usize = 100;

/// Whether `entry` is packed by [`pack_entry`]: an `i8` weight whose
/// output channel comes first (`OI`, `OHWI`) with a multiple of eight of
/// them.
pub fn packable(entry: &Entry<'_>) -> bool {
    entry.data_type == DataType::I8
        && matches!(entry.layout, "OI" | "OHWI")
        && entry.shape()[0].is_multiple_of(GROUP)
        && entry.shape()[0] > 0
}

/// The bytes a packed copy of `source` needs: as many as the file.
pub fn packed_len(source: &Blob<'_>) -> usize {
    source.bytes().len()
}

/// Copy the file header into `target`.
///
/// # Panics
///
/// When `target` is shorter than the file.
pub fn pack_header(source: &Blob<'_>, target: &mut [u8]) {
    assert!(target.len() >= packed_len(source), "packed copy too short");
    target[..HEADER_LEN].copy_from_slice(&source.bytes()[..HEADER_LEN]);
}

/// Copy entry `index` of `source` into `target`: its table-of-contents
/// entry (with the layout suffix when packed) and its data, packed when
/// [`packable`]. Call it for every entry, in any order, after
/// [`pack_header`]; the board does so with pauses between entries so
/// the other core keeps its share of the flash.
///
/// # Panics
///
/// When `target` is shorter than the file, or `index` is out of range.
pub fn pack_entry(source: &Blob<'_>, index: usize, target: &mut [u8]) {
    assert!(target.len() >= packed_len(source), "packed copy too short");
    let entry = source.entry(index);
    let start = HEADER_LEN + index * ENTRY_LEN;
    let toc = &mut target[start..start + ENTRY_LEN];
    toc.copy_from_slice(source.entry_bytes(index));
    let packed = packable(&entry);
    if packed {
        let layout = &mut target[start + LAYOUT_OFFSET..start + LAYOUT_OFFSET + LAYOUT_LEN];
        let len = entry.layout.len();
        assert!(
            len + PACKED_SUFFIX.len() <= LAYOUT_LEN,
            "layout string too long"
        );
        layout[len..len + PACKED_SUFFIX.len()].copy_from_slice(PACKED_SUFFIX.as_bytes());
    }
    let data = &mut target[entry.data_offset()..entry.data_offset() + entry.bytes().len()];
    if packed {
        let outputs = entry.shape()[0];
        let per_output = entry.element_count() / outputs;
        pack_rows(entry.bytes(), outputs, per_output, data);
    } else {
        data.copy_from_slice(entry.bytes());
    }
}

/// The whole copy at once, for the computer.
pub fn pack_all(source: &Blob<'_>, target: &mut [u8]) {
    pack_header(source, target);
    for index in 0..source.len() {
        pack_entry(source, index, target);
    }
}

/// Rearrange `rows` (`outputs` rows of `per_output` bytes) into
/// `packed`: `packed[(g * per_output + k) * 8 + j] = rows[(8 g + j) *
/// per_output + k]`.
///
/// # Panics
///
/// When the lengths do not match or `outputs` is not a multiple of eight.
pub fn pack_rows(rows: &[u8], outputs: usize, per_output: usize, packed: &mut [u8]) {
    assert!(outputs.is_multiple_of(GROUP), "pack_rows outputs");
    assert_eq!(rows.len(), outputs * per_output, "pack_rows rows");
    assert_eq!(packed.len(), rows.len(), "pack_rows packed");
    for (g, group) in packed.chunks_exact_mut(GROUP * per_output).enumerate() {
        for (k, slot) in group.chunks_exact_mut(GROUP).enumerate() {
            for (j, value) in slot.iter_mut().enumerate() {
                *value = rows[(g * GROUP + j) * per_output + k];
            }
        }
    }
}

/// The index of weight `k` of output `o` in a packed tensor whose
/// outputs have `per_output` weights each.
#[inline(always)]
pub fn packed_index(o: usize, k: usize, per_output: usize) -> usize {
    ((o / GROUP) * per_output + k) * GROUP + o % GROUP
}
