//! MFN_S8_V1's building block, two ways: with every tensor whole, and
//! fused band by band so that the wide tensors inside never leave a few
//! rows of buffer.
//!
//! The block takes `x` (`height x width x channels`), widens it with a
//! 1x1 convolution and PReLU, runs a depthwise 3x3 with PReLU over that
//! (stride 1, or 2 to halve the image), narrows it with a 1x1
//! convolution, and adds `x` when the block has a residual connection
//! (stride 1, as many channels out as in). The wide tensors are the large
//! ones: in the 14x14 stage, `x` is 25 KB and each wide tensor 50 KB.
//!
//! - [`run_whole`] computes each layer over the whole image before the
//!   next, so each wide tensor is written out whole and read back: in
//!   PSRAM on the board.
//! - [`run_banded`] computes the block a band of output rows at a time.
//!   Output row `r` needs the depthwise row `r`, which needs the wide
//!   rows around `stride * r`; the wide rows live in a ring of
//!   `stride * band + 2` rows, and each is computed once, just before it
//!   is first needed. The band's depthwise rows and its output (the
//!   residual added) are buffers of `band` rows too; the output goes to
//!   `output` in one copy, so the 1x1 kernel's sixteen-byte stores never
//!   land in PSRAM. Each 1x1 layer reads its weights once per band: on
//!   the board (`mfnbench-4`), bands of seven rows ran the 14x14 block in
//!   21 ms, single rows in 41 and whole tensors in 31, because the
//!   stage's 64 KB of 1x1 weights do not stay in the cache from one band
//!   to the next.
//!
//! Both compute exactly the same numbers.

use super::{Depthwise, Pointwise, Store, depthwise_row, pointwise};

/// One building block.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    /// Rows of the input.
    pub height: usize,
    /// Pixels per input row.
    pub width: usize,
    /// The widening 1x1 convolution, with its PReLU.
    pub expand: Pointwise<'a>,
    /// The depthwise 3x3 with its PReLU, stride 1 or 2.
    pub depthwise: Depthwise<'a>,
    /// The narrowing 1x1 convolution.
    pub project: Pointwise<'a>,
    /// Whether the input is added to the output.
    pub residual: bool,
}

impl Block<'_> {
    /// Channels of the input.
    pub fn channels(&self) -> usize {
        self.expand.input
    }

    /// Channels of the wide tensors.
    pub fn expanded(&self) -> usize {
        self.expand.output()
    }

    /// Channels of the output.
    pub fn outputs(&self) -> usize {
        self.project.output()
    }

    /// Rows of the output.
    pub fn out_height(&self) -> usize {
        self.depthwise.output_size(self.height)
    }

    /// Pixels per output row.
    pub fn out_width(&self) -> usize {
        self.depthwise.output_size(self.width)
    }

    /// The bytes of the output.
    pub fn output_len(&self) -> usize {
        self.out_height() * self.out_width() * self.outputs()
    }

    /// The bytes of the whole widened tensor, for [`run_whole`].
    pub fn wide_len(&self) -> usize {
        self.height * self.width * self.expanded()
    }

    /// The bytes of the whole depthwise output, for [`run_whole`].
    pub fn filtered_whole_len(&self) -> usize {
        self.out_height() * self.out_width() * self.expanded()
    }

    /// The bytes of the ring of wide rows, for [`run_banded`].
    pub fn ring_len(&self, band: usize) -> usize {
        (self.depthwise.stride * band + 2) * self.width * self.expanded()
    }

    /// The bytes of a band of depthwise rows, for [`run_banded`].
    pub fn filtered_len(&self, band: usize) -> usize {
        band * self.out_width() * self.expanded()
    }

    /// The bytes of a band of output rows, for [`run_banded`].
    pub fn staging_len(&self, band: usize) -> usize {
        band * self.out_width() * self.outputs()
    }

    /// The tallest band (at most the output's height) whose buffers fit
    /// in `ring`, `filtered` and `staging` bytes, or 0 when not even one
    /// row does.
    pub fn band_for(&self, ring: usize, filtered: usize, staging: usize) -> usize {
        (1..=self.out_height())
            .rev()
            .find(|&band| {
                self.ring_len(band) <= ring
                    && self.filtered_len(band) <= filtered
                    && self.staging_len(band) <= staging
            })
            .unwrap_or(0)
    }

    /// The products of one pass.
    pub fn products(&self) -> usize {
        let input = self.height * self.width;
        let output = self.out_height() * self.out_width();
        let expanded = self.expanded();
        input * self.channels() * expanded + output * expanded * (9 + self.outputs())
    }
}

/// Check that the block's layers fit together and with the buffers.
fn check(block: &Block<'_>, input: &[i8], output: &[i8]) {
    let expanded = block.expanded();
    assert_eq!(block.depthwise.channels(), expanded, "depthwise channels");
    assert_eq!(block.project.input, expanded, "projection input");
    if block.residual {
        assert_eq!(block.depthwise.stride, 1, "a residual needs stride 1");
        assert_eq!(
            block.outputs(),
            block.channels(),
            "a residual needs equal channels"
        );
    }
    assert_eq!(
        input.len(),
        block.height * block.width * block.channels(),
        "input size"
    );
    assert_eq!(output.len(), block.output_len(), "output size");
}

/// The wide rows above, at and below input row `centre`: `row(k)` gives
/// wide row `k`.
fn taps<'r>(
    centre: usize,
    height: usize,
    row: impl Fn(usize) -> &'r [i8],
) -> [Option<&'r [i8]>; 3] {
    [
        centre.checked_sub(1).map(&row),
        Some(row(centre)),
        (centre + 1 < height).then(|| row(centre + 1)),
    ]
}

/// The block on `input` into `output`, layer by layer over the whole
/// image: `wide` holds [`Block::wide_len`] bytes, `filtered`
/// [`Block::filtered_whole_len`].
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
    let wide = &mut wide[..block.wide_len()];
    let filtered = &mut filtered[..block.filtered_whole_len()];
    let row = block.width * block.expanded();
    let out_row = block.out_width() * block.expanded();
    pointwise(&block.expand, Store::Write, input, wide);
    let wide = &*wide;
    for (r, out) in filtered.chunks_exact_mut(out_row).enumerate() {
        let rows = taps(r * block.depthwise.stride, block.height, |k| {
            &wide[k * row..(k + 1) * row]
        });
        depthwise_row(&block.depthwise, rows, block.width, out);
    }
    if block.residual {
        output.copy_from_slice(input);
        pointwise(&block.project, Store::Add, filtered, output);
    } else {
        pointwise(&block.project, Store::Write, filtered, output);
    }
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
    let (height, width, stride) = (block.height, block.width, block.depthwise.stride);
    let in_row = width * block.channels();
    let wide_row = width * block.expanded();
    let filtered_row = block.out_width() * block.expanded();
    let out_row = block.out_width() * block.outputs();
    let slots = stride * band + 2;
    let ring = &mut ring[..slots * wide_row];
    // Wide rows computed so far: row `k` sits in slot `k % slots`.
    let mut ready = 0;
    let out_height = block.out_height();
    for first in (0..out_height).step_by(band) {
        let rows = band.min(out_height - first);
        // The band's wide rows and the one below the last, in runs of
        // consecutive slots.
        let last = (stride * (first + rows - 1) + 1).min(height - 1);
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
        let ring = &*ring;
        let slot = |k: usize| &ring[k % slots * wide_row..(k % slots + 1) * wide_row];
        for (i, out) in filtered[..rows * filtered_row]
            .chunks_exact_mut(filtered_row)
            .enumerate()
        {
            let rows = taps(stride * (first + i), height, slot);
            depthwise_row(&block.depthwise, rows, width, out);
        }
        let span = first * out_row..(first + rows) * out_row;
        let stage = &mut staging[..rows * out_row];
        let source = &filtered[..rows * filtered_row];
        if block.residual {
            stage.copy_from_slice(&input[span.clone()]);
            pointwise(&block.project, Store::Add, source, stage);
        } else {
            pointwise(&block.project, Store::Write, source, stage);
        }
        output[span].copy_from_slice(stage);
    }
}
