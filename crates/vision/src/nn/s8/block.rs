//! MFN_S8_V1's building block, two ways: with every tensor whole, and
//! fused band by band so that the wide tensors inside never leave a few
//! rows of buffer.
//!
//! The block takes `x` (`height x width x channels`), widens it with a
//! 1x1 convolution and PReLU to `expanded` channels (twice as many),
//! runs a depthwise 3x3 with PReLU over that, narrows it back with a 1x1
//! convolution and adds `x` (the residual). The wide tensors are the
//! large ones: in the 14x14 stage, `x` is 25 KB and each wide tensor
//! 50 KB.
//!
//! - [`run_whole`] computes each layer over the whole image before the
//!   next, so each wide tensor is written out whole and read back: in
//!   PSRAM on the board.
//! - [`run_banded`] computes the block a band of output rows at a time.
//!   Output row `r` needs the depthwise row `r`, which needs the wide
//!   rows `r - 1`, `r` and `r + 1`; the wide rows live in a ring of
//!   `band + 2` rows, and each is computed once, just before it is first
//!   needed. The band's depthwise rows and its output (the residual
//!   added) are buffers of `band` rows too; the output goes to `output`
//!   in one copy, so the 1x1 kernel's sixteen-byte stores never land in
//!   PSRAM. Each 1x1 layer reads its weights once per band: on the
//!   board, `mfnbench-3` found a single row per band slower than the
//!   whole tensors in the 14x14 stage, whose 64 KB of 1x1 weights do not
//!   stay in the cache from one row to the next.
//!
//! Both compute exactly the same numbers.

use super::{Depthwise, Pointwise, Store, depthwise_row, pointwise};

/// One building block: stride 1, with the residual add.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    /// Rows of the image.
    pub height: usize,
    /// Pixels per row.
    pub width: usize,
    /// The widening 1x1 convolution, with its PReLU.
    pub expand: Pointwise<'a>,
    /// The depthwise 3x3, stride 1, with its PReLU.
    pub depthwise: Depthwise<'a>,
    /// The narrowing 1x1 convolution, without a PReLU.
    pub project: Pointwise<'a>,
}

impl Block<'_> {
    /// Channels of the input and the output.
    pub fn channels(&self) -> usize {
        self.expand.input
    }

    /// Channels of the wide tensors.
    pub fn expanded(&self) -> usize {
        self.expand.output()
    }

    /// The bytes of a whole wide tensor, for [`run_whole`].
    pub fn whole_len(&self) -> usize {
        self.height * self.width * self.expanded()
    }

    /// The bytes of the ring of wide rows, for [`run_banded`].
    pub fn ring_len(&self, band: usize) -> usize {
        (band + 2) * self.width * self.expanded()
    }

    /// The bytes of a band of depthwise rows, for [`run_banded`].
    pub fn filtered_len(&self, band: usize) -> usize {
        band * self.width * self.expanded()
    }

    /// The bytes of a band of output rows, for [`run_banded`].
    pub fn staging_len(&self, band: usize) -> usize {
        band * self.width * self.channels()
    }

    /// The products of one pass.
    pub fn products(&self) -> usize {
        let pixels = self.height * self.width;
        let (channels, expanded) = (self.channels(), self.expanded());
        pixels * (2 * channels * expanded + 9 * expanded)
    }
}

/// Check that the block's layers fit together and with the buffers.
fn check(block: &Block<'_>, input: &[i8], output: &[i8]) {
    let (channels, expanded) = (block.channels(), block.expanded());
    assert_eq!(block.depthwise.channels, expanded, "depthwise channels");
    assert_eq!(block.depthwise.stride, 1, "a stride-1 block");
    assert_eq!(block.project.input, expanded, "projection input");
    assert_eq!(block.project.output(), channels, "projection output");
    let len = block.height * block.width * channels;
    assert_eq!(input.len(), len, "input size");
    assert_eq!(output.len(), len, "output size");
}

/// The block on `input` into `output`, layer by layer over the whole
/// image: `wide` and `filtered` hold the two wide tensors
/// ([`Block::whole_len`] bytes each).
///
/// # Panics
///
/// When the layers or the buffers do not fit together.
pub fn run_whole(
    block: &Block<'_>,
    input: &[i8],
    wide: &mut [i8],
    filtered: &mut [i8],
    output: &mut [i8],
) {
    check(block, input, output);
    let row = block.width * block.expanded();
    pointwise(&block.expand, Store::Write, input, wide);
    for (r, out) in filtered.chunks_exact_mut(row).enumerate() {
        let rows = [
            r.checked_sub(1).map(|r| &wide[r * row..(r + 1) * row]),
            Some(&wide[r * row..(r + 1) * row]),
            (r + 1 < block.height).then(|| &wide[(r + 1) * row..(r + 2) * row]),
        ];
        depthwise_row(&block.depthwise, rows, block.width, out);
    }
    output.copy_from_slice(input);
    pointwise(&block.project, Store::Add, filtered, output);
}

/// The block on `input` into `output`, `band` output rows at a time:
/// `ring` holds [`Block::ring_len`] bytes, `filtered`
/// [`Block::filtered_len`] and `staging` [`Block::staging_len`].
///
/// # Panics
///
/// When the layers or the buffers do not fit together, or `band` is 0.
pub fn run_banded(
    block: &Block<'_>,
    band: usize,
    input: &[i8],
    ring: &mut [i8],
    filtered: &mut [i8],
    staging: &mut [i8],
    output: &mut [i8],
) {
    check(block, input, output);
    assert!(band > 0, "a band of at least one row");
    let (height, width) = (block.height, block.width);
    let in_row = width * block.channels();
    let wide_row = width * block.expanded();
    let slots = band + 2;
    let ring = &mut ring[..slots * wide_row];
    // Wide rows computed so far: row `k` sits in slot `k % slots`.
    let mut ready = 0;
    for first in (0..height).step_by(band) {
        let rows = band.min(height - first);
        // The band's wide rows and the one below it, in runs of
        // consecutive slots.
        let last = (first + rows).min(height - 1);
        while ready <= last {
            let slot = ready % slots;
            let count = (last + 1 - ready).min(slots - slot);
            pointwise(
                &block.expand,
                Store::Write,
                &input[ready * in_row..(ready + count) * in_row],
                &mut ring[slot * wide_row..(slot + count) * wide_row],
            );
            ready += count;
        }
        let slot = |k: usize| &ring[k % slots * wide_row..(k % slots + 1) * wide_row];
        for (i, out) in filtered[..rows * wide_row]
            .chunks_exact_mut(wide_row)
            .enumerate()
        {
            let r = first + i;
            let rows = [
                r.checked_sub(1).map(slot),
                Some(slot(r)),
                (r + 1 < height).then(|| slot(r + 1)),
            ];
            depthwise_row(&block.depthwise, rows, width, out);
        }
        let span = first * in_row..(first + rows) * in_row;
        let stage = &mut staging[..rows * in_row];
        stage.copy_from_slice(&input[span.clone()]);
        pointwise(
            &block.project,
            Store::Add,
            &filtered[..rows * wide_row],
            stage,
        );
        output[span].copy_from_slice(stage);
    }
}
