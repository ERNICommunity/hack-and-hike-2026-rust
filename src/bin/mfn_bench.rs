//! A benchmark of the 8-bit kernels (`nn::s8`): is Espressif's
//! MFN_S8_V1 face recognizer fast enough for this board?
//!
//! MFN_S8_V1 is MIT-licensed, unlike EdgeFace-XXS. On the computer it
//! recognizes faces as well as EdgeFace-XXS does (LFW 99.27 % against
//! 99.42 %), but it needs 221 million products per face, more than twice
//! as many. `mfnbench-3` measured its two most frequent building blocks
//! at 276 ms for 134 million products, the best of whole tensors in PSRAM
//! and fusion one row at a time: about 450 ms for the network, against
//! 383 ms for EdgeFace-XXS. One-row fusion lost in the 14x14 stage, whose
//! 64 KB of 1x1 weights were read again for every row.
//!
//! This build fuses in bands of several rows (`s8::block::run_banded`):
//! the weights are read once per band, and each band's output collects
//! in internal RAM before one copy to PSRAM. What it does, all on CPU0,
//! logged line by line:
//!
//! 1. Probes single 8-bit instructions on known values (as before).
//! 2. Runs the kernels with the epilogues the network needs, on random
//!    data, each against the scalar model; the depthwise 3x3 once with
//!    its rows in PSRAM and once in internal RAM.
//! 3. Runs the two block shapes (six of 14x14 with 128 channels widened
//!    to 256, four of 28x28 with 64 widened to 128) layer by layer with
//!    the wide tensors whole in PSRAM, and banded with bands of 1, 2, 4
//!    and 7 rows in internal RAM (as many as fit). Each is checked
//!    against the scalar model; the best of three runs counts.
//! 4. Projects the whole network from the best way of each shape (an
//!    estimate).
//!
//! Build with `cargo dist --bin mfn_bench`, flash, and read the log.

#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use hack_and_hike::{
    Board,
    capabilities::display::{SCREEN, SIZE},
    logging, psram,
    ui::{Canvas, common, theme},
};
use hack_and_hike_vision::nn::{
    check,
    s8::{
        self, Depthwise, LANES, Plan, Pointwise, Prelu, Store,
        block::{self, Block},
        model, probe,
    },
};
use log::{info, warn};

esp_bootloader_esp_idf::esp_app_desc!();

/// Which build this is, in the first log line.
const BUILD_ID: &str = "mfnbench-4";

/// The CPU clock, for cycles per product.
const CPU_MHZ: u64 = 240;

/// All products of MFN_S8_V1 for one face.
const NETWORK_PRODUCTS: u64 = 221_200_000;

/// What EdgeFace-XXS takes alone on this board (`faceid-14`), in ms.
const EDGEFACE_MS: u64 = 383;

/// Timed runs per measurement; the fastest counts.
const RUNS: usize = 3;

/// One building block shape of MFN_S8_V1, and how many blocks have it.
struct Shape {
    /// Rows and pixels per row.
    side: usize,
    /// Channels of the input and the output.
    channels: usize,
    /// Blocks of this shape in the network.
    count: u64,
}

/// The two most frequent block shapes: 135 million of the network's
/// 221 million products.
const SHAPES: [Shape; 2] = [
    Shape {
        side: 14,
        channels: 128,
        count: 6,
    },
    Shape {
        side: 28,
        channels: 64,
        count: 4,
    },
];

/// The bytes of the largest block input (and output).
const MAX_TENSOR: usize = 28 * 28 * 64;
/// The bytes of the largest wide tensor.
const MAX_WIDE: usize = 28 * 28 * 128;
/// The bytes of the largest wide row (28 pixels of 128 channels, and
/// 14 of 256).
const MAX_ROW: usize = 28 * 128;
/// The bytes of the largest output row (28 pixels of 64 channels, and
/// 14 of 128).
const MAX_OUT_ROW: usize = 28 * 64;
/// The band heights to measure, as far as internal RAM holds them.
const BANDS: [usize; 4] = [1, 2, 4, 7];
/// The most channels of any layer here.
const MAX_CHANNELS: usize = 256;

/// `len` values in PSRAM, starting on a 16-byte boundary.
fn aligned_psram<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let spare = 16 / core::mem::size_of::<T>().max(1);
    let raw = psram::leaked_slice::<T>(len + spare, value);
    let skip = raw.as_ptr().align_offset(16);
    &mut raw[skip..skip + len]
}

/// `len` bytes of internal RAM, with room to start them on a 16-byte
/// boundary, or nothing when no free block is that large: the radio,
/// audio and IMU tasks take their share before `main` runs.
fn try_internal(len: usize) -> Option<alloc::vec::Vec<i8>> {
    let mut raw = alloc::vec::Vec::new();
    raw.try_reserve_exact(len + 16).ok()?;
    raw.resize(len + 16, 0i8);
    Some(raw)
}

/// `raw` (from [`try_internal`]) kept for good, from its first 16-byte
/// boundary.
fn leak_aligned(raw: alloc::vec::Vec<i8>) -> &'static mut [i8] {
    let len = raw.len() - 16;
    let raw = raw.leak();
    let skip = raw.as_ptr().align_offset(16);
    &mut raw[skip..skip + len]
}

/// Random `i8` values in `-64..64`, as the sums of real layers stay well
/// inside a 20-bit lane with them.
fn fill(seed: u32, values: &mut [i8]) {
    check::noise(seed, values);
    for value in values.iter_mut() {
        *value >>= 1;
    }
}

/// The fastest of [`RUNS`] runs of `run`, in microseconds.
fn best_of(mut run: impl FnMut()) -> u64 {
    (0..RUNS)
        .map(|_| {
            let started = Instant::now();
            run();
            started.elapsed().as_micros()
        })
        .min()
        .unwrap_or(0)
}

/// Cycles per product, in hundredths, for `micros` over `products`.
fn centicycles(micros: u64, products: u64) -> u64 {
    (micros * CPU_MHZ * 100).checked_div(products).unwrap_or(0)
}

/// `micros` and its cycles per product, as `"<us> us (<c.cc> cyc/product)"`.
struct Timing(u64, u64);

impl core::fmt::Display for Timing {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let c = centicycles(self.0, self.1);
        write!(f, "{} us ({}.{:02} cyc/product)", self.0, c / 100, c % 100)
    }
}

/// Write `text` in the middle of the screen.
fn show(display: &mut hack_and_hike::capabilities::display::Display, text: &str) {
    let mut canvas = Canvas::new(SIZE);
    canvas.clear(theme::WHITE);
    common::centered_text(
        &mut canvas,
        SCREEN,
        text,
        common::TITLE_FONT,
        theme::CHARCOAL,
    );
    canvas.show(&mut display.surface(SCREEN));
}

/// Run the instruction probes and log what they found. `true` when every
/// assumption of the kernels held.
fn probe_instructions() -> bool {
    let mut held = true;
    let ramp: [i32; LANES] = core::array::from_fn(|i| i as i32 + 1);

    let (raw, lanes) = probe::lane_layout();
    let expected = s8::encode_lanes(&ramp);
    let ramp_i8 = ramp.map(|v| v as i8);
    if raw == expected && lanes == ramp_i8 {
        info!("probe lanes: ok, lane i at bit 20 i, lanes 0..7 low, 8..15 high");
    } else {
        held = false;
        warn!(
            "probe lanes: DIFFERENT accumulator {raw:08x?} expected {expected:08x?}; out {lanes:?}"
        );
    }

    let values: [i32; LANES] = core::array::from_fn(|i| i as i32 * 7 - 56);
    let lanes = probe::shift_lanes(&values, 0);
    if lanes == values.map(|v| v as i8) {
        info!("probe image: ok, the plan's image loads as encoded");
    } else {
        held = false;
        warn!("probe image: DIFFERENT {lanes:?} expected {values:?}");
    }

    let odd = [3, -3, 5, -5, 1, -1, 7, -7, 2, -2, 6, -6, 0, 4, -4, 9];
    let lanes = probe::shift_lanes(&odd, 1);
    let floor = odd.map(|v| (v >> 1) as i8);
    let rounded = odd.map(|v| ((v + 1) >> 1) as i8);
    if lanes == floor {
        info!("probe shift: ok, floor (the plan's half makes it round)");
    } else if lanes == rounded {
        held = false;
        warn!("probe shift: ROUNDS by itself; the plans must not add half: {lanes:?}");
    } else {
        held = false;
        warn!("probe shift: UNEXPECTED {lanes:?} for {odd:?} >> 1");
    }

    let big = [
        300, -300, 200_000, -200_000, 127, -128, 128, -129, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let lanes = probe::shift_lanes(&big, 0);
    let saturated = big.map(|v| v.clamp(-128, 127) as i8);
    if lanes == saturated {
        info!("probe saturation: ok");
    } else {
        held = false;
        warn!("probe saturation: DIFFERENT {lanes:?} expected {saturated:?}");
    }

    let (step, lanes) = probe::broadcast_step();
    if step == 2 && lanes == [11; LANES] {
        info!("probe broadcast: ok, each broadcast load steps one byte");
    } else {
        held = false;
        warn!(
            "probe broadcast: DIFFERENT, moved {step} bytes, loaded {lanes:?} (expected 2, all 11)"
        );
    }

    let control = probe::overflow(32);
    let over = probe::overflow(40);
    let kind = match over[0] {
        78 => "lanes wider than 20 bits",
        -50 => "20-bit lanes that wrap",
        63 => "20-bit lanes that saturate",
        _ => "something else",
    };
    info!(
        "probe overflow: 32 x 16129 -> {} (expected 63), 40 x 16129 -> {}: {kind}",
        control[0], over[0]
    );
    held
}

/// One layer's random weights, biases, slopes and plans, in PSRAM.
struct Layer {
    /// The weights in the kernels' layout.
    weights: &'static mut [i8],
    /// The biases.
    bias: &'static mut [i32],
    /// The PReLU slopes.
    alpha: &'static mut [i8],
    /// The plans.
    plans: &'static mut [Plan],
}

impl Layer {
    /// Room for a layer of up to `weights` weights and `channels` outputs.
    fn allocate(weights: usize, channels: usize) -> Self {
        Self {
            weights: aligned_psram(weights, 0),
            bias: aligned_psram(channels, 0),
            alpha: aligned_psram(channels, 0),
            plans: aligned_psram(channels / LANES, Plan::ZERO),
        }
    }

    /// Random values for `taps` inputs per output and `channels`
    /// outputs, and their plans; the parts in use.
    fn randomize(
        &mut self,
        seed: u32,
        taps: usize,
        channels: usize,
        shift: u32,
        prelu: Option<(u32, u32)>,
    ) -> (&[i8], &[i32], &[i8], &[Plan]) {
        let weights = &mut self.weights[..taps * channels];
        fill(seed, weights);
        let mut noise = [0i8; MAX_CHANNELS];
        check::noise(seed + 1, &mut noise);
        let bias = &mut self.bias[..channels];
        for (b, n) in bias.iter_mut().zip(noise) {
            *b = i32::from(n) * 16;
        }
        let alpha = &mut self.alpha[..channels];
        fill(seed + 2, alpha);
        let plans = &mut self.plans[..channels / LANES];
        let activation = prelu.map(|(positive, shift)| Prelu {
            alpha,
            positive,
            shift,
        });
        s8::plans(bias, shift, activation.as_ref(), plans);
        (weights, bias, alpha, plans)
    }
}

/// [`block::run_whole`] in scalar code: the reference.
fn reference(
    block: &Block<'_>,
    input: &[i8],
    wide: &mut [i8],
    filtered: &mut [i8],
    out: &mut [i8],
) {
    let row = block.width * block.expanded();
    model::pointwise(&block.expand, Store::Write, input, wide);
    for (r, filtered_row) in filtered.chunks_exact_mut(row).enumerate() {
        let rows = [
            r.checked_sub(1).map(|r| &wide[r * row..(r + 1) * row]),
            Some(&wide[r * row..(r + 1) * row]),
            (r + 1 < block.height).then(|| &wide[(r + 1) * row..(r + 2) * row]),
        ];
        model::depthwise_row(&block.depthwise, rows, block.width, filtered_row);
    }
    out.copy_from_slice(input);
    model::pointwise(&block.project, Store::Add, filtered, out);
}

/// The PSRAM buffers of the block runs.
struct Tensors {
    /// The block input.
    input: &'static mut [i8],
    /// The reference output.
    expected: &'static mut [i8],
    /// The output under test.
    actual: &'static mut [i8],
    /// The first wide tensor (or the ring in PSRAM).
    wide: &'static mut [i8],
    /// The second wide tensor (or the row in PSRAM).
    filtered: &'static mut [i8],
}

/// The banded runs' buffers in internal RAM, for bands of up to `band`
/// rows.
struct Internal {
    /// The tallest band they hold.
    band: usize,
    /// `band + 2` wide rows.
    ring: &'static mut [i8],
    /// `band` depthwise rows.
    filtered: &'static mut [i8],
    /// `band` output rows.
    staging: &'static mut [i8],
}

impl Internal {
    /// The buffers for the tallest band of [`BANDS`] that internal RAM
    /// holds, or nothing.
    fn allocate() -> Option<Self> {
        BANDS.iter().rev().find_map(|&band| {
            let ring = try_internal((band + 2) * MAX_ROW)?;
            let filtered = try_internal(band * MAX_ROW)?;
            let staging = try_internal(band * MAX_OUT_ROW)?;
            Some(Self {
                band,
                ring: leak_aligned(ring),
                filtered: leak_aligned(filtered),
                staging: leak_aligned(staging),
            })
        })
    }
}

/// Whether `actual` equals `expected`; logs the first difference.
fn same(what: &str, expected: &[i8], actual: &[i8]) -> bool {
    match expected.iter().zip(actual).position(|(e, a)| e != a) {
        None => true,
        Some(first) => {
            let wrong = expected.iter().zip(actual).filter(|(e, a)| e != a).count();
            warn!(
                "  {what}: {wrong} of {} values DIFFER, first at {first}: {} expected {}",
                expected.len(),
                actual[first],
                expected[first]
            );
            false
        }
    }
}

/// Times and checks the three kernels with their epilogues on the 14x14
/// stage's sizes. `true` when all three matched the model.
fn measure_kernels(
    tensors: &mut Tensors,
    layers: &mut [Layer; 3],
    internal: Option<&mut Internal>,
) -> bool {
    let (pixels, channels, expanded) = (196usize, 128usize, 256usize);
    let [expand, depth, project] = layers;
    let mut matched = true;

    let (weights, bias, alpha, plans) = expand.randomize(10, channels, expanded, 8, Some((0, 7)));
    let layer = Pointwise {
        input: channels,
        weights,
        bias,
        plans,
        shift: 8,
        prelu: Some(Prelu {
            alpha,
            positive: 0,
            shift: 7,
        }),
    };
    let input = &mut tensors.input[..pixels * channels];
    fill(20, input);
    let expected = &mut tensors.wide[..pixels * expanded];
    model::pointwise(&layer, Store::Write, input, expected);
    let actual = &mut tensors.filtered[..pixels * expanded];
    let micros = best_of(|| s8::pointwise(&layer, Store::Write, input, actual));
    let ok = same("1x1 + PReLU", expected, actual);
    matched &= ok;
    let products = (pixels * channels * expanded) as u64;
    info!(
        "kernel 1x1 196x128->256 + PReLU: {}, {}",
        Timing(micros, products),
        if ok { "bit-exact" } else { "WRONG" }
    );

    let (weights, bias, alpha, plans) = depth.randomize(30, 9, expanded, 6, Some((1, 7)));
    let layer = Depthwise {
        channels: expanded,
        weights,
        bias,
        plans,
        shift: 6,
        stride: 1,
        prelu: Some(Prelu {
            alpha,
            positive: 1,
            shift: 7,
        }),
    };
    let source = &*expected;
    let row = 14 * expanded;
    let run = |out: &mut [i8], kernel: bool| {
        for (r, out_row) in out.chunks_exact_mut(row).enumerate() {
            let rows = [
                r.checked_sub(1).map(|r| &source[r * row..(r + 1) * row]),
                Some(&source[r * row..(r + 1) * row]),
                (r + 1 < 14).then(|| &source[(r + 1) * row..(r + 2) * row]),
            ];
            if kernel {
                s8::depthwise_row(&layer, rows, 14, out_row);
            } else {
                model::depthwise_row(&layer, rows, 14, out_row);
            }
        }
    };
    let expected_dw = &mut tensors.expected[..pixels * expanded];
    run(expected_dw, false);
    let actual_dw = &mut tensors.actual[..pixels * expanded];
    let micros = best_of(|| run(actual_dw, true));
    let ok = same("depthwise + PReLU", expected_dw, actual_dw);
    matched &= ok;
    let products = (pixels * 9 * expanded) as u64;
    info!(
        "kernel depthwise 14x14x256 + PReLU: {}, {}",
        Timing(micros, products),
        if ok { "bit-exact" } else { "WRONG" }
    );

    if let Some(internal) = internal {
        // The same layer on rows in internal RAM: random wide rows in the
        // ring, `band` output rows into `filtered`.
        let band = internal.band;
        let rows_in = (band + 2).min(14);
        let rows_out = rows_in - 2;
        let source = &mut internal.ring[..rows_in * row];
        fill(35, source);
        let source = &*source;
        let run = |out: &mut [i8], kernel: bool| {
            for (i, out_row) in out.chunks_exact_mut(row).enumerate() {
                let r = i + 1;
                let rows = [
                    Some(&source[(r - 1) * row..r * row]),
                    Some(&source[r * row..(r + 1) * row]),
                    Some(&source[(r + 1) * row..(r + 2) * row]),
                ];
                if kernel {
                    s8::depthwise_row(&layer, rows, 14, out_row);
                } else {
                    model::depthwise_row(&layer, rows, 14, out_row);
                }
            }
        };
        let expected_dw = &mut tensors.expected[..rows_out * row];
        run(expected_dw, false);
        let actual_dw = &mut internal.filtered[..rows_out * row];
        let micros = best_of(|| run(actual_dw, true));
        let ok = same("depthwise in internal RAM", expected_dw, actual_dw);
        matched &= ok;
        let products = (rows_out * 14 * 9 * expanded) as u64;
        info!(
            "kernel depthwise {rows_out}x14x256 + PReLU, rows in internal RAM: {}, {}",
            Timing(micros, products),
            if ok { "bit-exact" } else { "WRONG" }
        );
    }

    let (weights, bias, _, plans) = project.randomize(40, expanded, channels, 9, None);
    let layer = Pointwise {
        input: expanded,
        weights,
        bias,
        plans,
        shift: 9,
        prelu: None,
    };
    let source = &tensors.expected[..pixels * expanded];
    let expected = &mut tensors.wide[..pixels * channels];
    fill(50, expected);
    let actual = &mut tensors.filtered[..pixels * channels];
    actual.copy_from_slice(expected);
    model::pointwise(&layer, Store::Add, source, expected);
    let started = Instant::now();
    s8::pointwise(&layer, Store::Add, source, actual);
    let micros = started.elapsed().as_micros();
    let ok = same("1x1 + add", expected, actual);
    matched &= ok;
    let products = (pixels * channels * expanded) as u64;
    info!(
        "kernel 1x1 196x256->128 + add: {} (one run), {}",
        Timing(micros, products),
        if ok { "bit-exact" } else { "WRONG" }
    );
    matched
}

/// Times and checks one block shape three ways. Returns the best fused
/// time in microseconds, the block's products and whether every way
/// matched the model.
fn measure_block(
    index: usize,
    shape: &Shape,
    tensors: &mut Tensors,
    layers: &mut [Layer; 3],
    internal: Option<&mut Internal>,
) -> (u64, u64, bool) {
    let (side, channels) = (shape.side, shape.channels);
    let expanded = 2 * channels;
    let seed = 100 + index as u32 * 10;
    let [expand, depth, project] = layers;
    let (ew, eb, ea, ep) = expand.randomize(seed, channels, expanded, 8, Some((0, 7)));
    let (dw, db, da, dp) = depth.randomize(seed + 3, 9, expanded, 6, Some((1, 7)));
    let (pw, pb, _, pp) = project.randomize(seed + 6, expanded, channels, 9, None);
    let block = Block {
        height: side,
        width: side,
        expand: Pointwise {
            input: channels,
            weights: ew,
            bias: eb,
            plans: ep,
            shift: 8,
            prelu: Some(Prelu {
                alpha: ea,
                positive: 0,
                shift: 7,
            }),
        },
        depthwise: Depthwise {
            channels: expanded,
            weights: dw,
            bias: db,
            plans: dp,
            shift: 6,
            stride: 1,
            prelu: Some(Prelu {
                alpha: da,
                positive: 1,
                shift: 7,
            }),
        },
        project: Pointwise {
            input: expanded,
            weights: pw,
            bias: pb,
            plans: pp,
            shift: 9,
            prelu: None,
        },
    };
    let len = side * side * channels;
    let input = &mut tensors.input[..len];
    fill(seed + 9, input);
    let expected = &mut tensors.expected[..len];
    let actual = &mut tensors.actual[..len];
    let whole = block.whole_len();
    reference(
        &block,
        input,
        &mut tensors.wide[..whole],
        &mut tensors.filtered[..whole],
        expected,
    );
    let products = block.products() as u64;
    let mut matched = true;

    actual.fill(0);
    let micros_whole = best_of(|| {
        block::run_whole(
            &block,
            input,
            &mut tensors.wide[..whole],
            &mut tensors.filtered[..whole],
            actual,
        );
    });
    let ok = same("whole", expected, actual);
    matched &= ok;
    info!(
        "block {side}x{side}x{channels}->{expanded} (x{}) whole, wide tensors in PSRAM: {}, {}",
        shape.count,
        Timing(micros_whole, products),
        if ok { "bit-exact" } else { "WRONG" }
    );

    let mut best = micros_whole;
    if let Some(internal) = internal {
        for band in BANDS.into_iter().filter(|&band| band <= internal.band) {
            actual.fill(0);
            let micros = best_of(|| {
                block::run_banded(
                    &block,
                    band,
                    input,
                    &mut internal.ring[..block.ring_len(band)],
                    &mut internal.filtered[..block.filtered_len(band)],
                    &mut internal.staging[..block.staging_len(band)],
                    actual,
                );
            });
            let ok = same("banded", expected, actual);
            matched &= ok;
            info!(
                "  bands of {band} rows in internal RAM: {}, {}",
                Timing(micros, products),
                if ok { "bit-exact" } else { "WRONG" }
            );
            best = best.min(micros);
        }
    }
    (best, products, matched)
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board { mut display, .. } = Board::init();
    info!("=== mfn bench [{BUILD_ID}] ===");
    show(&mut display, "MFN benchmark running");

    logging::report_memory("mfn bench start");
    let mut internal = Internal::allocate();
    match &internal {
        Some(internal) => info!("internal RAM holds bands of up to {} rows", internal.band),
        None => warn!("not enough free internal RAM: no banded runs"),
    }
    let mut tensors = Tensors {
        input: aligned_psram(MAX_TENSOR, 0),
        expected: aligned_psram(MAX_WIDE, 0),
        actual: aligned_psram(MAX_WIDE, 0),
        wide: aligned_psram(MAX_WIDE, 0),
        filtered: aligned_psram(MAX_WIDE, 0),
    };
    let mut layers = [
        Layer::allocate(128 * 256, MAX_CHANNELS),
        Layer::allocate(9 * 256, MAX_CHANNELS),
        Layer::allocate(256 * 128, MAX_CHANNELS),
    ];
    logging::report_memory("mfn bench ready");

    let probes_held = probe_instructions();
    let mut matched = measure_kernels(&mut tensors, &mut layers, internal.as_mut());
    Timer::after(Duration::from_millis(20)).await;

    let mut measured_micros = 0u64;
    let mut measured_products = 0u64;
    for (index, shape) in SHAPES.iter().enumerate() {
        let (micros, products, ok) =
            measure_block(index, shape, &mut tensors, &mut layers, internal.as_mut());
        matched &= ok;
        measured_micros += micros * shape.count;
        measured_products += products * shape.count;
        Timer::after(Duration::from_millis(20)).await;
    }

    let rest = NETWORK_PRODUCTS - measured_products;
    let rest_micros = rest * measured_micros / measured_products;
    info!(
        "summary: the {} blocks measured: {} ms for {} M products ({}.{:02} cyc/product, best way each)",
        SHAPES.iter().map(|s| s.count).sum::<u64>(),
        measured_micros / 1000,
        measured_products / 1_000_000,
        centicycles(measured_micros, measured_products) / 100,
        centicycles(measured_micros, measured_products) % 100,
    );
    info!(
        "summary: ESTIMATE for the whole network at that rate: {} ms (+{} ms for the other {} M products); EdgeFace-XXS alone: {EDGEFACE_MS} ms",
        (measured_micros + rest_micros) / 1000,
        rest_micros / 1000,
        rest / 1_000_000,
    );
    info!(
        "summary: probes {}, kernels and blocks {}, {} scalar fallbacks",
        if probes_held {
            "held"
        } else {
            "FAILED (see above)"
        },
        if matched {
            "bit-exact"
        } else {
            "WRONG somewhere (see above)"
        },
        s8::fallbacks()
    );
    show(&mut display, "MFN benchmark done: see the log");
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
