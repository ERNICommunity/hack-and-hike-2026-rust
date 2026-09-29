//! Image views over borrowed buffers, and box downscaling of a camera frame.
//!
//! Nothing here owns memory. The caller has a buffer (for the board, in
//! PSRAM) and lends it to a view: [`Image`] for reading, [`ImageMut`] for
//! writing. The views know the width, the height and the number of channels
//! of the pixels, and check every index, so a wrong coordinate panics
//! instead of reading the wrong pixel silently.
//!
//! A camera frame is different: the board's camera driver hands out one row
//! at a time, not one contiguous slice. So the source of a frame is the
//! [`Rgb565Source`] trait. [`Rgb565Frame`] implements it for a frame that
//! does lie in one slice, which the tests and the desktop tools use.
//!
//! The downscaling functions shrink a 320x240 frame to the size a neural
//! network expects, for example 80x60 with a factor of 4. They average each
//! block of source pixels (a box filter), which also removes some noise.
//!
//! The application scales every frame down, 76,800 pixels each time, so
//! [`downscale_to_rgb`] has a fast path for the factors up to 4: it looks
//! the two bytes of a pixel up in two tables and adds the three colours
//! of a pixel with one addition (see `PackedRgb`). It gives the same
//! bytes as the plain loop, which the tests check.

use core::ops::Range;

use crate::pixel::{self, BYTES_PER_PIXEL};

/// A source of big-endian RGB565 rows, each `width * 2` bytes long.
///
/// This is a trait and not a slice because the board's camera frame only
/// exposes its rows one at a time.
pub trait Rgb565Source {
    /// Number of pixels in a row.
    fn width(&self) -> usize;
    /// Number of rows.
    fn height(&self) -> usize;
    /// The bytes of row `y`: `width() * 2` bytes, two per pixel.
    fn row(&self, y: usize) -> &[u8];
}

/// An [`Rgb565Source`] over one contiguous slice of big-endian RGB565
/// pixels, row after row.
#[derive(Clone, Copy, Debug)]
pub struct Rgb565Frame<'a> {
    /// The pixel bytes, `width * height * 2` of them.
    data: &'a [u8],
    /// Number of pixels in a row.
    width: usize,
    /// Number of rows.
    height: usize,
}

impl<'a> Rgb565Frame<'a> {
    /// A frame over `data`, which holds `width * height` pixels of two bytes
    /// each.
    ///
    /// # Panics
    ///
    /// When `data.len()` is not `width * height * 2`.
    pub fn new(data: &'a [u8], width: usize, height: usize) -> Self {
        assert_eq!(
            data.len(),
            width * height * BYTES_PER_PIXEL,
            "frame buffer length matches {width}x{height} RGB565 pixels"
        );
        Self {
            data,
            width,
            height,
        }
    }
}

impl Rgb565Source for Rgb565Frame<'_> {
    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn row(&self, y: usize) -> &[u8] {
        let stride = self.width * BYTES_PER_PIXEL;
        &self.data[y * stride..(y + 1) * stride]
    }
}

/// The byte range of row `y` of an image with `width` pixels of `channels`
/// bytes each.
fn row_range(width: usize, channels: usize, y: usize) -> core::ops::Range<usize> {
    let stride = width * channels;
    y * stride..(y + 1) * stride
}

/// The byte range of pixel `(x, y)` of an image with `width` pixels of
/// `channels` bytes each. Checks `x` against the width: without the check,
/// an `x` past the end of a row would silently address the next row.
fn pixel_range(width: usize, channels: usize, x: usize, y: usize) -> core::ops::Range<usize> {
    assert!(x < width, "x {x} is inside the width {width}");
    let start = (y * width + x) * channels;
    start..start + channels
}

/// A read-only view of an image with `CHANNELS` bytes per pixel, stored row
/// after row without padding.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a, const CHANNELS: usize> {
    /// The pixel bytes, `width * height * CHANNELS` of them.
    data: &'a [u8],
    /// Number of pixels in a row.
    width: usize,
    /// Number of rows.
    height: usize,
}

/// A writable view of an image with `CHANNELS` bytes per pixel, stored row
/// after row without padding.
#[derive(Debug)]
pub struct ImageMut<'a, const CHANNELS: usize> {
    /// The pixel bytes, `width * height * CHANNELS` of them.
    data: &'a mut [u8],
    /// Number of pixels in a row.
    width: usize,
    /// Number of rows.
    height: usize,
}

/// A read-only 8-bit gray image.
pub type GrayImage<'a> = Image<'a, 1>;
/// A writable 8-bit gray image.
pub type GrayImageMut<'a> = ImageMut<'a, 1>;
/// A read-only 8-bit RGB image, bytes in the order red, green, blue.
pub type RgbImage<'a> = Image<'a, 3>;
/// A writable 8-bit RGB image, bytes in the order red, green, blue.
pub type RgbImageMut<'a> = ImageMut<'a, 3>;

impl<'a, const CHANNELS: usize> Image<'a, CHANNELS> {
    /// A view over `data`, which holds `width * height` pixels of `CHANNELS`
    /// bytes each.
    ///
    /// # Panics
    ///
    /// When `data.len()` is not `width * height * CHANNELS`.
    pub fn new(data: &'a [u8], width: usize, height: usize) -> Self {
        assert_eq!(
            data.len(),
            width * height * CHANNELS,
            "image buffer length matches {width}x{height} pixels of {CHANNELS} bytes"
        );
        Self {
            data,
            width,
            height,
        }
    }

    /// Number of pixels in a row.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The bytes of row `y`: `width() * CHANNELS` of them.
    ///
    /// # Panics
    ///
    /// When `y` is not below the height.
    pub fn row(&self, y: usize) -> &[u8] {
        &self.data[row_range(self.width, CHANNELS, y)]
    }

    /// The channel values of pixel `(x, y)`.
    ///
    /// # Panics
    ///
    /// When `x` is not below the width or `y` is not below the height.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; CHANNELS] {
        let mut values = [0; CHANNELS];
        values.copy_from_slice(&self.data[pixel_range(self.width, CHANNELS, x, y)]);
        values
    }

    /// All pixel bytes, row after row.
    pub fn data(&self) -> &[u8] {
        self.data
    }
}

impl<'a, const CHANNELS: usize> ImageMut<'a, CHANNELS> {
    /// A view over `data`, which holds `width * height` pixels of `CHANNELS`
    /// bytes each.
    ///
    /// # Panics
    ///
    /// When `data.len()` is not `width * height * CHANNELS`.
    pub fn new(data: &'a mut [u8], width: usize, height: usize) -> Self {
        assert_eq!(
            data.len(),
            width * height * CHANNELS,
            "image buffer length matches {width}x{height} pixels of {CHANNELS} bytes"
        );
        Self {
            data,
            width,
            height,
        }
    }

    /// Number of pixels in a row.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The bytes of row `y`: `width() * CHANNELS` of them.
    ///
    /// # Panics
    ///
    /// When `y` is not below the height.
    pub fn row(&self, y: usize) -> &[u8] {
        &self.data[row_range(self.width, CHANNELS, y)]
    }

    /// The bytes of row `y` for writing: `width() * CHANNELS` of them.
    ///
    /// # Panics
    ///
    /// When `y` is not below the height.
    pub fn row_mut(&mut self, y: usize) -> &mut [u8] {
        &mut self.data[row_range(self.width, CHANNELS, y)]
    }

    /// The channel values of pixel `(x, y)`.
    ///
    /// # Panics
    ///
    /// When `x` is not below the width or `y` is not below the height.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; CHANNELS] {
        self.as_image().pixel(x, y)
    }

    /// Write the channel values of pixel `(x, y)`.
    ///
    /// # Panics
    ///
    /// When `x` is not below the width or `y` is not below the height.
    pub fn set_pixel(&mut self, x: usize, y: usize, values: [u8; CHANNELS]) {
        self.data[pixel_range(self.width, CHANNELS, x, y)].copy_from_slice(&values);
    }

    /// All pixel bytes, row after row.
    pub fn data(&self) -> &[u8] {
        self.data
    }

    /// A read-only view of the same image, for the functions that take an
    /// [`Image`].
    pub fn as_image(&self) -> Image<'_, CHANNELS> {
        Image {
            data: self.data,
            width: self.width,
            height: self.height,
        }
    }
}

/// Shrink `src` by `factor` in both directions into `dst`, with `convert`
/// turning each source pixel into the destination's channels first. Each
/// destination pixel in `columns` and `rows` is the rounded mean of a
/// `factor x factor` block; the others are left as they are.
///
/// The sums are per destination pixel: the loop visits the `factor` source
/// rows and `factor` source columns of one block, then moves on. That needs
/// no row of sums (which would have to live somewhere) and is fast enough
/// for the factors of 2 and 4 in use.
///
/// # Panics
///
/// See [`downscale_to_gray`].
fn downscale<const CHANNELS: usize>(
    src: &impl Rgb565Source,
    factor: usize,
    dst: &mut ImageMut<'_, CHANNELS>,
    columns: Range<usize>,
    rows: Range<usize>,
    convert: impl Fn([u8; BYTES_PER_PIXEL]) -> [u8; CHANNELS],
) {
    assert!(factor >= 1, "downscale factor is at least 1");
    check_sizes(src, factor, dst.width(), dst.height());
    // Number of source pixels in one block. `factor` is tiny, so the cast
    // cannot overflow.
    let count = (factor * factor) as u32;
    for dst_y in clip(rows, dst.height()) {
        for dst_x in clip(columns.clone(), dst.width()) {
            let mut sums = [0u32; CHANNELS];
            for src_y in dst_y * factor..(dst_y + 1) * factor {
                let row = src.row(src_y);
                let block =
                    &row[dst_x * factor * BYTES_PER_PIXEL..(dst_x + 1) * factor * BYTES_PER_PIXEL];
                for bytes in block.chunks_exact(BYTES_PER_PIXEL) {
                    let values = convert([bytes[0], bytes[1]]);
                    for (sum, value) in sums.iter_mut().zip(values) {
                        *sum += u32::from(value);
                    }
                }
            }
            let mut mean = [0u8; CHANNELS];
            for (out, sum) in mean.iter_mut().zip(sums) {
                // The sum is at most `count * 255`, so the mean fits in a u8.
                *out = ((sum + count / 2) / count) as u8;
            }
            dst.set_pixel(dst_x, dst_y, mean);
        }
    }
}

/// Check that a destination of `width` x `height` is `src` shrunk by
/// `factor`.
///
/// # Panics
///
/// See [`downscale_to_gray`].
fn check_sizes(src: &impl Rgb565Source, factor: usize, width: usize, height: usize) {
    assert!(
        src.width().is_multiple_of(factor) && src.height().is_multiple_of(factor),
        "source size {}x{} is a multiple of the factor {factor}",
        src.width(),
        src.height()
    );
    assert!(
        width == src.width() / factor && height == src.height() / factor,
        "destination size {width}x{height} is the source size {}x{} divided by {factor}",
        src.width(),
        src.height()
    );
}

/// The part of `range` below `len`; an empty range when there is none,
/// also when `range` runs backwards.
fn clip(range: Range<usize>, len: usize) -> Range<usize> {
    let start = range.start.min(len);
    start..range.end.min(len).max(start)
}

/// Shrink an RGB565 frame by `factor` in both directions into a gray image.
///
/// Every source pixel is converted to gray first. Then each destination
/// pixel is the rounded mean `(sum + n / 2) / n` of the `n = factor * factor`
/// grays of its block. So a frame of 320x240 with a factor of 4 gives an
/// 80x60 gray image.
///
/// # Panics
///
/// - When `factor` is 0.
/// - When the source width or height is not a multiple of `factor`.
/// - When `dst` is not exactly `width / factor` by `height / factor`.
pub fn downscale_to_gray(src: &impl Rgb565Source, factor: usize, dst: &mut GrayImageMut<'_>) {
    let (width, height) = (dst.width(), dst.height());
    downscale(src, factor, dst, 0..width, 0..height, |bytes| {
        [pixel::rgb565_be_to_gray(bytes)]
    });
}

/// Shrink an RGB565 frame by `factor` in both directions into an RGB image.
///
/// Like [`downscale_to_gray`], but every source pixel is expanded to 8-bit
/// red, green and blue and each channel is averaged on its own.
///
/// # Panics
///
/// The same as [`downscale_to_gray`].
pub fn downscale_to_rgb(src: &impl Rgb565Source, factor: usize, dst: &mut RgbImageMut<'_>) {
    let (width, height) = (dst.width(), dst.height());
    downscale_to_rgb_within(src, factor, dst, 0..width, 0..height);
}

/// [`downscale_to_rgb`] for the destination pixels in `columns` and `rows`
/// only; the others are left as they are. Each pixel it writes has the
/// value `downscale_to_rgb` gives it.
///
/// For a computation that reads a part of the scaled-down image, such as
/// the face alignment (`align::source_region` says which part).
///
/// # Panics
///
/// The same as [`downscale_to_gray`].
pub fn downscale_to_rgb_within(
    src: &impl Rgb565Source,
    factor: usize,
    dst: &mut RgbImageMut<'_>,
    columns: Range<usize>,
    rows: Range<usize>,
) {
    match factor {
        1 => downscale_to_rgb_packed::<1>(src, dst, columns, rows),
        2 => downscale_to_rgb_packed::<2>(src, dst, columns, rows),
        3 => downscale_to_rgb_packed::<3>(src, dst, columns, rows),
        4 => downscale_to_rgb_packed::<4>(src, dst, columns, rows),
        _ => downscale(src, factor, dst, columns, rows, pixel::rgb565_be_to_rgb888),
    }
}

/// [`downscale_to_rgb`] as a plain loop over the pixels, for any factor:
/// what the fast path is checked against.
///
/// # Panics
///
/// The same as [`downscale_to_gray`].
pub fn downscale_to_rgb_plain(src: &impl Rgb565Source, factor: usize, dst: &mut RgbImageMut<'_>) {
    let (width, height) = (dst.width(), dst.height());
    downscale(
        src,
        factor,
        dst,
        0..width,
        0..height,
        pixel::rgb565_be_to_rgb888,
    );
}

/// The three colours of an RGB565 pixel as 8-bit values, side by side in
/// one `u32`, from two table lookups.
///
/// A packed value holds red in bits 20 to 29, green in bits 10 to 19 and
/// blue in bits 0 to 9: ten bits each. Adding packed values adds the
/// three colours at once, and ten bits hold the sum of up to four 8-bit
/// values (4 x 255 = 1020), so the sums never run into each other.
///
/// The first byte of a pixel holds red and the upper three bits of green,
/// the second byte the lower three bits of green and blue (see
/// [`pixel`]). The 8-bit green is the 6-bit value shifted left by two,
/// plus its top two bits: the shifted part is the sum of what each byte
/// contributes, and the top two bits come from the first byte alone. So
/// the pixel's packed value is `high[first] + low[second]`.
struct PackedRgb {
    /// What the first byte of a pixel contributes.
    high: [u32; 256],
    /// What the second byte contributes.
    low: [u32; 256],
}

impl PackedRgb {
    /// Bits per colour in a packed value.
    const BITS: u32 = 10;
    /// The mask of one colour.
    const MASK: u32 = (1 << Self::BITS) - 1;
    /// The most pixels whose packed values may be added.
    const MAX_SUM: usize = 4;

    /// Build the two tables: about 500 short steps, against the 76,800
    /// pixels of a frame.
    fn new() -> Self {
        let mut tables = Self {
            high: [0; 256],
            low: [0; 256],
        };
        for byte in 0..=255u8 {
            // The byte as the first of a pixel whose second byte is zero,
            // and the other way round: each colour is the sum of the two.
            let [red, green_high, _] = pixel::rgb565_be_to_rgb888([byte, 0]);
            let [_, green_low, blue] = pixel::rgb565_be_to_rgb888([0, byte]);
            tables.high[usize::from(byte)] =
                (u32::from(red) << (2 * Self::BITS)) | (u32::from(green_high) << Self::BITS);
            tables.low[usize::from(byte)] = (u32::from(green_low) << Self::BITS) | u32::from(blue);
        }
        tables
    }

    /// The sum of the packed values of the pixels in `bytes`, at most
    /// [`PackedRgb::MAX_SUM`] of them.
    #[inline(always)]
    fn sum(&self, bytes: &[u8]) -> u32 {
        let mut sum = 0;
        for pixel in bytes.chunks_exact(BYTES_PER_PIXEL) {
            sum += self.high[usize::from(pixel[0])] + self.low[usize::from(pixel[1])];
        }
        sum
    }
}

/// [`downscale_to_rgb_within`] for a factor of at most 4, known when the
/// code is compiled: the division by the pixel count is by a constant (a
/// shift for 1, 2 and 4), and the loops over a block have a fixed length.
///
/// # Panics
///
/// The same as [`downscale_to_gray`].
fn downscale_to_rgb_packed<const FACTOR: usize>(
    src: &impl Rgb565Source,
    dst: &mut RgbImageMut<'_>,
    columns: Range<usize>,
    rows: Range<usize>,
) {
    const {
        assert!(FACTOR >= 1 && FACTOR <= PackedRgb::MAX_SUM);
    }
    check_sizes(src, FACTOR, dst.width(), dst.height());
    let columns = clip(columns, dst.width());
    let tables = PackedRgb::new();
    let count = (FACTOR * FACTOR) as u32;
    let block_bytes = FACTOR * BYTES_PER_PIXEL;
    for dst_y in clip(rows, dst.height()) {
        let rows: [&[u8]; FACTOR] = core::array::from_fn(|row| src.row(dst_y * FACTOR + row));
        let out_row = &mut dst.row_mut(dst_y)[columns.start * 3..columns.end * 3];
        for (dst_x, out) in columns.clone().zip(out_row.chunks_exact_mut(3)) {
            let (mut red, mut green, mut blue) = (0u32, 0u32, 0u32);
            for row in rows {
                // One row of the block: at most four pixels, so their
                // packed sum fits.
                let sum = tables.sum(&row[dst_x * block_bytes..(dst_x + 1) * block_bytes]);
                red += sum >> (2 * PackedRgb::BITS);
                green += (sum >> PackedRgb::BITS) & PackedRgb::MASK;
                blue += sum & PackedRgb::MASK;
            }
            // The sums are at most `count * 255`, so the means fit a byte.
            out[0] = ((red + count / 2) / count) as u8;
            out[1] = ((green + count / 2) / count) as u8;
            out[2] = ((blue + count / 2) / count) as u8;
        }
    }
}

/// Convert an RGB image to gray, pixel by pixel (`pixel::rgb888_to_gray`).
///
/// # Panics
///
/// When the images differ in size.
pub fn rgb_to_gray(src: &RgbImage<'_>, dst: &mut GrayImageMut<'_>) {
    assert_eq!(
        (src.width(), src.height()),
        (dst.width(), dst.height()),
        "rgb_to_gray sizes"
    );
    for (gray, rgb) in dst.data.iter_mut().zip(src.data.chunks_exact(3)) {
        *gray = pixel::rgb888_to_gray([rgb[0], rgb[1], rgb[2]]);
    }
}
