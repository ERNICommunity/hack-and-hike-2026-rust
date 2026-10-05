//! The arithmetic of the 8-bit kernels in scalar code: exactly what the
//! vector unit computes (the sums of MFN_S8_V1 never come near the 20
//! bits of a lane).

use super::{Depthwise, LANES, Pointwise, Prelu, Store};

/// Half of `2^shift` (zero without a shift): added before the shift so
/// that it rounds half up.
pub const fn half(shift: u32) -> i32 {
    if shift == 0 { 0 } else { 1 << (shift - 1) }
}

/// `sum` (bias and products, in product units) in output units:
/// shifted right by `shift`, rounded half up, saturated to `i8`.
pub fn requantize(sum: i64, shift: u32) -> i8 {
    ((sum + i64::from(half(shift))) >> shift).clamp(-128, 127) as i8
}

/// Channel `channel`'s PReLU of `value`.
pub fn prelu(prelu: &Prelu<'_>, channel: usize, value: i8) -> i8 {
    if value >= 0 {
        (i32::from(value) << prelu.positive).min(127) as i8
    } else {
        requantize(
            i64::from(value) * i64::from(prelu.alpha[channel]),
            prelu.shift,
        )
    }
}

/// A finished sum of channel `channel` through the requantization, the
/// PReLU and the store.
fn finish(
    sum: i64,
    shift: u32,
    activation: Option<&Prelu<'_>>,
    channel: usize,
    store: Store,
    target: &mut i8,
) {
    let mut value = requantize(sum, shift);
    if let Some(activation) = activation {
        value = prelu(activation, channel, value);
    }
    *target = match store {
        Store::Write => value,
        Store::Add => target.saturating_add(value),
    };
}

/// [`super::pointwise`] in scalar code.
pub fn pointwise(layer: &Pointwise<'_>, store: Store, input: &[i8], output: &mut [i8]) {
    let (channels, outputs) = (layer.input, layer.output());
    for (pixel, out) in input
        .chunks_exact(channels)
        .zip(output.chunks_exact_mut(outputs))
    {
        for (o, target) in out.iter_mut().enumerate() {
            let filter = &layer.weights[o / LANES * channels * LANES..];
            let lane = o % LANES;
            // In `i32`: an `i64` multiply is a library call on the board,
            // and 131,000 channels of `i8` products would still fit.
            let mut sum = 0i32;
            for (c, &x) in pixel.iter().enumerate() {
                sum += i32::from(x) * i32::from(filter[c * LANES + lane]);
            }
            finish(
                i64::from(layer.bias[o]) + i64::from(sum),
                layer.shift,
                layer.prelu.as_ref(),
                o,
                store,
                target,
            );
        }
    }
}

/// [`super::depthwise_row`] in scalar code.
pub fn depthwise_row(
    layer: &Depthwise<'_>,
    rows: [Option<&[i8]>; 3],
    width: usize,
    output: &mut [i8],
) {
    let channels = layer.channels;
    for (ox, out) in output.chunks_exact_mut(channels).enumerate() {
        let centre = ox * layer.stride;
        for (c, target) in out.iter_mut().enumerate() {
            let filter = &layer.weights[c / LANES * 9 * LANES..];
            let lane = c % LANES;
            let mut sum = 0i32;
            for (ky, row) in rows.iter().enumerate() {
                let Some(row) = row else { continue };
                for kx in 0..3 {
                    let Some(x) = (centre + kx).checked_sub(1).filter(|&x| x < width) else {
                        continue;
                    };
                    sum += i32::from(row[x * channels + c])
                        * i32::from(filter[(ky * 3 + kx) * LANES + lane]);
                }
            }
            finish(
                i64::from(layer.bias[c]) + i64::from(sum),
                layer.shift,
                layer.prelu.as_ref(),
                c,
                Store::Write,
                target,
            );
        }
    }
}
