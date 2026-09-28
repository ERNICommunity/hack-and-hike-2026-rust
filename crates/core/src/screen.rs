//! The wire format of the live screen feed.
//!
//! The firmware sends a copy of the 320x240 panel over the USB serial port,
//! mixed with the log text. The autoflash page shows it. This module holds
//! the parts of the format that need no hardware: the packet writers, the
//! pixel run encoder, COBS and the checksum. The decoders are here for the
//! tests; they mirror the TypeScript decoders in `autoflash/src/`.
//!
//! # Framing
//!
//! Log lines are plain UTF-8 text, so a serial terminal still shows them. A
//! screen packet is one `0x00` byte, the COBS-encoded body, and one more
//! `0x00` byte. COBS (Consistent Overhead Byte Stuffing) removes every zero
//! byte from the body, so `0x00` marks the packet edges without doubt. Text
//! never contains `0x00`: the logger replaces it with a space.
//!
//! # Body
//!
//! All integers are little-endian.
//!
//! | Bytes | Contents |
//! | --- | --- |
//! | 1 | [`MAGIC`], never valid UTF-8 |
//! | 1 | kind: [`KIND_HELLO`] or [`KIND_RECT`] |
//! | ... | the fields of the kind |
//! | 2 | CRC-16/CCITT-FALSE over every byte before it, see [`crc16`] |
//!
//! - Hello: `version u8` ([`VERSION`]), `width u16`, `height u16`. The host
//!   clears its image to black.
//! - Rect: `x u16`, `y u16`, `w u16`, `h u16`, then `w * h` pixels in row
//!   order as pixel runs (see [`encode_runs`]). At most [`MAX_RECT_PIXELS`]
//!   pixels.
//!
//! # Host to device
//!
//! Every byte the host sends is a refresh request. The firmware then sends
//! Hello and the whole screen.

/// The first body byte of every packet. It is never valid UTF-8, so text
/// is never mistaken for a packet.
pub const MAGIC: u8 = 0xFE;
/// Kind byte of a Hello packet: format version and screen size.
pub const KIND_HELLO: u8 = 0x01;
/// Kind byte of a Rect packet: the pixels of one rectangle.
pub const KIND_RECT: u8 = 0x02;
/// The format version that Hello carries.
pub const VERSION: u8 = 1;
/// The byte that starts and ends every packet on the wire.
pub const DELIMITER: u8 = 0x00;
/// Most pixels in one Rect packet (1,536 bytes of raw RGB565).
pub const MAX_RECT_PIXELS: usize = 768;
/// Size of one packet buffer on the device. The largest Rect fits with its
/// header, the run control bytes, the CRC, the COBS overhead and both
/// delimiters.
pub const SLOT_BYTES: usize = 1664;
/// Size of the CRC at the end of the body.
const CRC_BYTES: usize = 2;
/// Size of the Rect header: magic, kind and four `u16` fields.
const RECT_HEADER_BYTES: usize = 10;
/// Most pixels in one literal run, and most copies in one repeat run.
const MAX_RUN: usize = 128;
/// Where a packet writer puts the body before it encodes it in place. The
/// gap to the COBS output (which starts at byte 1) must be at least one
/// plus one byte for every 254 body bytes; 16 bytes cover bodies of up to
/// 3,556 bytes.
const BODY_OFFSET: usize = 16;

/// CRC-16/CCITT-FALSE of `data`: polynomial `0x1021`, start value
/// `0xFFFF`, no bit reflection, no final XOR.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// COBS-encode the `len` bytes at `buf[src..]` into `buf[dst..]`, in the
/// same buffer. Return the number of encoded bytes, or `None` when they do
/// not fit or the gap is too small.
///
/// The output grows by one byte, plus one byte for every 254 non-zero
/// bytes in a row. So the output never overtakes the input when
/// `src - dst` is at least `1 + len / 254`.
pub fn cobs_encode_in_place(buf: &mut [u8], src: usize, len: usize, dst: usize) -> Option<usize> {
    if src < dst || src - dst < 1 + len / 254 || src.checked_add(len)? > buf.len() {
        return None;
    }
    // The byte that holds the length of the current block, and the count
    // it will get. It is written when the block ends.
    let mut code_at = dst;
    let mut code: u8 = 1;
    let mut write = dst + 1;
    for read in src..src + len {
        let byte = buf[read];
        if byte == 0 {
            buf[code_at] = code;
            code_at = write;
            code = 1;
        } else {
            *buf.get_mut(write)? = byte;
            code += 1;
            if code == 0xFF {
                buf[code_at] = code;
                write += 1;
                code_at = write;
                code = 1;
            }
        }
        write += 1;
        if write > buf.len() {
            return None;
        }
    }
    *buf.get_mut(code_at)? = code;
    Some(write - dst)
}

/// Decode the COBS bytes of one packet (without the delimiters) into `out`.
/// Return the number of decoded bytes, or `None` when the input is not
/// valid COBS or `out` is too small.
pub fn cobs_decode(input: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut read = 0;
    let mut write = 0;
    while read < input.len() {
        let code = input[read];
        if code == 0 {
            return None;
        }
        read += 1;
        let data = usize::from(code) - 1;
        let block = input.get(read..read + data)?;
        if block.contains(&0) {
            return None;
        }
        out.get_mut(write..write + data)?.copy_from_slice(block);
        read += data;
        write += data;
        if code != 0xFF && read < input.len() {
            *out.get_mut(write)? = 0;
            write += 1;
        }
    }
    Some(write)
}

/// Encode `count` pixels as runs into `out`. `pixel(i)` returns pixel `i`
/// as RGB565; it is called once for each pixel, in order. Return the
/// number of bytes written, or `None` when `out` is too small.
///
/// Each run starts with a control byte `c`:
///
/// - `c < 128`: `c + 1` literal pixels follow, two bytes each, big-endian.
/// - `c >= 128`: one pixel follows (two bytes, big-endian); it repeats
///   `c - 127` times.
///
/// Two or more equal pixels become a repeat run. In the worst case, the
/// output is one byte per 128 pixels larger than the raw pixels.
pub fn encode_runs(
    count: usize,
    mut pixel: impl FnMut(usize) -> u16,
    out: &mut [u8],
) -> Option<usize> {
    let mut writer = RunWriter {
        out,
        len: 0,
        literal_at: None,
        literal_len: 0,
    };
    // The pending run: a value and how often it repeats so far.
    let mut value = 0;
    let mut repeats = 0;
    for index in 0..count {
        let next = pixel(index);
        if repeats > 0 && next == value && repeats < MAX_RUN {
            repeats += 1;
        } else {
            writer.run(value, repeats)?;
            value = next;
            repeats = 1;
        }
    }
    writer.run(value, repeats)?;
    Some(writer.len)
}

/// The output side of [`encode_runs`].
struct RunWriter<'a> {
    /// The output buffer.
    out: &'a mut [u8],
    /// Bytes written so far.
    len: usize,
    /// Position of the control byte of the open literal run, if one is open.
    literal_at: Option<usize>,
    /// Pixels in the open literal run.
    literal_len: usize,
}

impl RunWriter<'_> {
    /// Append one byte.
    fn byte(&mut self, byte: u8) -> Option<()> {
        *self.out.get_mut(self.len)? = byte;
        self.len += 1;
        Some(())
    }

    /// Append one pixel, big-endian.
    fn pixel(&mut self, value: u16) -> Option<()> {
        let [high, low] = value.to_be_bytes();
        self.byte(high)?;
        self.byte(low)
    }

    /// Append `repeats` copies of `value`: as a repeat run when there are
    /// two or more, else as one more pixel of the literal run.
    fn run(&mut self, value: u16, repeats: usize) -> Option<()> {
        match repeats {
            0 => Some(()),
            1 => {
                let at = match self.literal_at {
                    Some(at) if self.literal_len < MAX_RUN => at,
                    _ => {
                        let at = self.len;
                        self.byte(0)?;
                        self.literal_at = Some(at);
                        self.literal_len = 0;
                        at
                    }
                };
                self.literal_len += 1;
                self.out[at] = (self.literal_len - 1) as u8;
                self.pixel(value)
            }
            _ => {
                self.literal_at = None;
                self.byte((repeats + 127) as u8)?;
                self.pixel(value)
            }
        }
    }
}

/// Decode pixel runs from `input` until `out` is full. Return the number of
/// input bytes used, or `None` when the runs are cut short or would
/// overfill `out`.
pub fn decode_runs(input: &[u8], out: &mut [u16]) -> Option<usize> {
    let mut read = 0;
    let mut filled = 0;
    while filled < out.len() {
        let control = *input.get(read)?;
        read += 1;
        if control < 128 {
            let count = usize::from(control) + 1;
            let bytes = input.get(read..read + count * 2)?;
            let target = out.get_mut(filled..filled + count)?;
            for (slot, pair) in target.iter_mut().zip(bytes.chunks_exact(2)) {
                *slot = u16::from_be_bytes([pair[0], pair[1]]);
            }
            read += count * 2;
            filled += count;
        } else {
            let count = usize::from(control) - 127;
            let pair = input.get(read..read + 2)?;
            out.get_mut(filled..filled + count)?
                .fill(u16::from_be_bytes([pair[0], pair[1]]));
            read += 2;
            filled += count;
        }
    }
    Some(read)
}

/// Add the CRC after `body_len` body bytes at `out[BODY_OFFSET..]`, then
/// COBS-encode the body in place and add the delimiters. Return the length
/// of the whole packet.
fn finish(out: &mut [u8], body_len: usize) -> Option<usize> {
    let body_end = BODY_OFFSET + body_len;
    let crc = crc16(out.get(BODY_OFFSET..body_end)?);
    out.get_mut(body_end..body_end + CRC_BYTES)?
        .copy_from_slice(&crc.to_le_bytes());
    *out.first_mut()? = DELIMITER;
    let encoded = cobs_encode_in_place(out, BODY_OFFSET, body_len + CRC_BYTES, 1)?;
    *out.get_mut(1 + encoded)? = DELIMITER;
    Some(encoded + 2)
}

/// Write a whole Hello packet, delimiters included, into `out`. Return its
/// length, or `None` when `out` is too small.
pub fn write_hello(width: u16, height: u16, out: &mut [u8]) -> Option<usize> {
    let [w0, w1] = width.to_le_bytes();
    let [h0, h1] = height.to_le_bytes();
    let body = [MAGIC, KIND_HELLO, VERSION, w0, w1, h0, h1];
    out.get_mut(BODY_OFFSET..BODY_OFFSET + body.len())?
        .copy_from_slice(&body);
    finish(out, body.len())
}

/// A rectangle of the screen, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left column.
    pub x: u16,
    /// Top row.
    pub y: u16,
    /// Width.
    pub w: u16,
    /// Height.
    pub h: u16,
}

impl Rect {
    /// Number of pixels in the rectangle.
    pub const fn pixels(&self) -> usize {
        self.w as usize * self.h as usize
    }
}

/// Write a whole Rect packet, delimiters included, into `out`. `pixel(i)`
/// returns pixel `i` of the rectangle in row order, as RGB565. Return the
/// packet's length, or `None` when the rectangle has more than
/// [`MAX_RECT_PIXELS`] pixels or `out` is too small.
///
/// A buffer of [`SLOT_BYTES`] always fits a rectangle of
/// [`MAX_RECT_PIXELS`] pixels.
pub fn write_rect(rect: Rect, pixel: impl FnMut(usize) -> u16, out: &mut [u8]) -> Option<usize> {
    if rect.pixels() > MAX_RECT_PIXELS {
        return None;
    }
    let mut header = [0; RECT_HEADER_BYTES];
    header[0] = MAGIC;
    header[1] = KIND_RECT;
    for (field, value) in header[2..]
        .chunks_exact_mut(2)
        .zip([rect.x, rect.y, rect.w, rect.h])
    {
        field.copy_from_slice(&value.to_le_bytes());
    }
    out.get_mut(BODY_OFFSET..BODY_OFFSET + RECT_HEADER_BYTES)?
        .copy_from_slice(&header);
    let runs_at = BODY_OFFSET + RECT_HEADER_BYTES;
    let runs = encode_runs(rect.pixels(), pixel, out.get_mut(runs_at..)?)?;
    finish(out, RECT_HEADER_BYTES + runs)
}

/// A checked packet body, as [`parse_body`] returns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packet<'a> {
    /// The screen size and format version.
    Hello {
        /// Format version, [`VERSION`] today.
        version: u8,
        /// Screen width in pixels.
        width: u16,
        /// Screen height in pixels.
        height: u16,
    },
    /// New pixels for one rectangle.
    Rect {
        /// Where the pixels go.
        rect: Rect,
        /// The pixel runs; decode them with [`decode_runs`].
        runs: &'a [u8],
    },
}

/// Check the magic and the CRC of a COBS-decoded body and split it into its
/// fields. `None` for a damaged body or an unknown kind.
pub fn parse_body(body: &[u8]) -> Option<Packet<'_>> {
    let crc_at = body.len().checked_sub(CRC_BYTES)?;
    let (data, crc) = body.split_at(crc_at);
    if crc16(data).to_le_bytes() != crc || data.first() != Some(&MAGIC) {
        return None;
    }
    let field = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
    match *data.get(1)? {
        KIND_HELLO => Some(Packet::Hello {
            version: *data.get(2)?,
            width: field(3)?,
            height: field(5)?,
        }),
        KIND_RECT => Some(Packet::Rect {
            rect: Rect {
                x: field(2)?,
                y: field(4)?,
                w: field(6)?,
                h: field(8)?,
            },
            runs: data.get(RECT_HEADER_BYTES..)?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::{vec, vec::Vec};

    use super::*;

    /// COBS-encode `input` through the in-place encoder.
    fn cobs(input: &[u8]) -> Vec<u8> {
        let gap = 1 + input.len() / 254;
        let mut buf = vec![0xAA; gap + input.len()];
        buf[gap..].copy_from_slice(input);
        let len = cobs_encode_in_place(&mut buf, gap, input.len(), 0).expect("fits");
        buf.truncate(len);
        buf
    }

    /// Encode and decode `input` and check that it survives.
    fn cobs_round_trip(input: &[u8]) {
        let encoded = cobs(input);
        assert!(!encoded.contains(&0), "COBS output has no zero bytes");
        assert!(encoded.len() <= input.len() + 1 + input.len() / 254);
        let mut decoded = vec![0; input.len() + 8];
        let len = cobs_decode(&encoded, &mut decoded).expect("valid COBS");
        assert_eq!(&decoded[..len], input);
    }

    /// Encode and decode `pixels` as runs and check that they survive.
    fn runs_round_trip(pixels: &[u16]) -> usize {
        let mut out = vec![0; pixels.len() * 3 + 2];
        let len = encode_runs(pixels.len(), |i| pixels[i], &mut out).expect("fits");
        assert!(len <= pixels.len() * 2 + pixels.len().div_ceil(MAX_RUN));
        let mut decoded = vec![0; pixels.len()];
        assert_eq!(decode_runs(&out[..len], &mut decoded), Some(len));
        assert_eq!(decoded, pixels);
        len
    }

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn cobs_round_trips() {
        cobs_round_trip(&[]);
        cobs_round_trip(&[0]);
        cobs_round_trip(&[0, 0]);
        cobs_round_trip(&[0, 1, 2, 0]);
        cobs_round_trip(&[1, 2, 3]);
        cobs_round_trip(&[0xFE, 0x02, 0, 0, 5]);
        for len in [253, 254, 255, 508, 509, 1600] {
            let data: Vec<u8> = (0..len).map(|i| (i % 255 + 1) as u8).collect();
            cobs_round_trip(&data);
            let mut with_zeros = data.clone();
            with_zeros[0] = 0;
            *with_zeros.last_mut().unwrap() = 0;
            cobs_round_trip(&with_zeros);
        }
    }

    #[test]
    fn cobs_known_encoding() {
        assert_eq!(
            cobs(&[0x11, 0x22, 0x00, 0x33]),
            [0x03, 0x11, 0x22, 0x02, 0x33]
        );
        assert_eq!(cobs(&[0x00]), [0x01, 0x01]);
    }

    #[test]
    fn cobs_rejects_bad_input() {
        let mut out = [0; 16];
        assert_eq!(cobs_decode(&[0x05, 0x01], &mut out), None);
        assert_eq!(cobs_decode(&[0x02, 0x00], &mut out), None);
        assert_eq!(cobs_decode(&[0x00], &mut out), None);
    }

    #[test]
    fn in_place_encoder_refuses_a_small_gap() {
        let mut buf = [1; 300];
        assert_eq!(cobs_encode_in_place(&mut buf, 1, 260, 0), None);
    }

    #[test]
    fn runs_of_equal_pixels() {
        assert_eq!(runs_round_trip(&[0x1234; 5]), 3);
        assert_eq!(runs_round_trip(&[0; 128]), 3);
        // 300 equal pixels need three repeat runs: 128, 128 and 44.
        assert_eq!(runs_round_trip(&[0xF800; 300]), 9);
    }

    #[test]
    fn runs_of_different_pixels() {
        let pixels: Vec<u16> = (0..300).collect();
        // Literal runs of 128, 128 and 44 pixels.
        assert_eq!(runs_round_trip(&pixels), 3 + 600);
        runs_round_trip(&[1]);
        runs_round_trip(&[]);
    }

    #[test]
    fn runs_mixed() {
        let mut pixels = vec![7, 8, 8, 8, 9, 10, 10];
        pixels.extend(core::iter::repeat_n(0, 200));
        pixels.extend((0..140).map(|i| i * 3));
        runs_round_trip(&pixels);
    }

    #[test]
    fn a_rectangle_split_across_rows_decodes_in_row_order() {
        // Two rows of a 4-pixel-wide rectangle: the encoder does not care
        // about row edges, so a run may continue into the next row.
        let rows = [[1, 1, 2, 3], [3, 3, 3, 4]];
        let flat: Vec<u16> = rows.iter().flatten().copied().collect();
        runs_round_trip(&flat);
    }

    #[test]
    fn decode_runs_rejects_overflow_and_short_input() {
        let mut out = [0; 4];
        // A repeat of 5 into room for 4.
        assert_eq!(decode_runs(&[0x84, 0x00, 0x01], &mut out), None);
        // A literal of 2 with only one pixel.
        assert_eq!(decode_runs(&[0x01, 0x00, 0x01], &mut out), None);
    }

    /// Strip the delimiters, COBS-decode and parse a written packet.
    fn body_of(packet: &[u8]) -> Vec<u8> {
        assert_eq!(packet.first(), Some(&DELIMITER));
        assert_eq!(packet.last(), Some(&DELIMITER));
        let inner = &packet[1..packet.len() - 1];
        assert!(!inner.contains(&0));
        let mut body = vec![0; SLOT_BYTES];
        let len = cobs_decode(inner, &mut body).expect("valid COBS");
        body.truncate(len);
        body
    }

    #[test]
    fn hello_round_trip() {
        let mut slot = [0; SLOT_BYTES];
        let len = write_hello(320, 240, &mut slot).unwrap();
        let body = body_of(&slot[..len]);
        assert_eq!(
            parse_body(&body),
            Some(Packet::Hello {
                version: VERSION,
                width: 320,
                height: 240
            })
        );
    }

    #[test]
    fn rect_round_trip_worst_case_fits_a_slot() {
        let mut slot = [0; SLOT_BYTES];
        let rect = Rect {
            x: 10,
            y: 20,
            w: 256,
            h: 3,
        };
        // Pixels that never repeat, and many zero bytes for COBS.
        let pixel = |i: usize| (i as u16).wrapping_mul(0x0100);
        let len = write_rect(rect, pixel, &mut slot).expect("fits a slot");
        let body = body_of(&slot[..len]);
        let Some(Packet::Rect { rect: parsed, runs }) = parse_body(&body) else {
            panic!("not a rect");
        };
        assert_eq!(parsed, rect);
        let mut pixels = vec![0; rect.pixels()];
        assert_eq!(decode_runs(runs, &mut pixels), Some(runs.len()));
        assert!(pixels.iter().enumerate().all(|(i, &p)| p == pixel(i)));
    }

    #[test]
    fn rect_too_large_is_refused() {
        let mut slot = [0; SLOT_BYTES];
        let rect = Rect {
            x: 0,
            y: 0,
            w: 320,
            h: 3,
        };
        assert_eq!(write_rect(rect, |_| 0, &mut slot), None);
    }

    #[test]
    fn damaged_body_is_refused() {
        let mut slot = [0; SLOT_BYTES];
        let len = write_hello(320, 240, &mut slot).unwrap();
        let mut body = body_of(&slot[..len]);
        body[3] ^= 1;
        assert_eq!(parse_body(&body), None);
    }

    /// The packet that `autoflash/src/stream.test.ts` decodes too: a 3x2
    /// rectangle at (1, 2) with black, red and white pixels.
    const GOLDEN: &str = "0004fe02010202020302020101010383f80105ffffdf3c00";

    #[test]
    fn golden_packet() {
        let mut slot = [0; SLOT_BYTES];
        let pixels = [0x0000, 0xF800, 0xF800, 0xF800, 0xF800, 0xFFFF];
        let rect = Rect {
            x: 1,
            y: 2,
            w: 3,
            h: 2,
        };
        let len = write_rect(rect, |i| pixels[i], &mut slot).unwrap();
        let hex: std::string::String = slot[..len]
            .iter()
            .map(|b| std::format!("{b:02x}"))
            .collect();
        std::println!("golden packet: {hex}");
        assert_eq!(hex, GOLDEN);
    }
}
