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
/// destination pixel is the rounded mean of a `factor x factor` block.
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
    convert: impl Fn([u8; BYTES_PER_PIXEL]) -> [u8; CHANNELS],
) {
    assert!(factor >= 1, "downscale factor is at least 1");
    assert!(
        src.width().is_multiple_of(factor) && src.height().is_multiple_of(factor),
        "source size {}x{} is a multiple of the factor {factor}",
        src.width(),
        src.height()
    );
    assert!(
        dst.width() == src.width() / factor && dst.height() == src.height() / factor,
        "destination size {}x{} is the source size {}x{} divided by {factor}",
        dst.width(),
        dst.height(),
        src.width(),
        src.height()
    );
    // Number of source pixels in one block. `factor` is tiny, so the cast
    // cannot overflow.
    let count = (factor * factor) as u32;
    for dst_y in 0..dst.height() {
        for dst_x in 0..dst.width() {
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
    downscale(src, factor, dst, |bytes| [pixel::rgb565_be_to_gray(bytes)]);
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
    downscale(src, factor, dst, pixel::rgb565_be_to_rgb888);
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
    for y in 0..src.height() {
        for (x, rgb) in src.row(y).chunks_exact(3).enumerate() {
            dst.set_pixel(
                x,
                y,
                [crate::pixel::rgb888_to_gray([rgb[0], rgb[1], rgb[2]])],
            );
        }
    }
}
