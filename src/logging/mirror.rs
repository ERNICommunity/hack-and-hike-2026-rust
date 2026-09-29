//! The screen side of the live screen feed: a copy of the panel, sent as
//! packets.
//!
//! The display transport reports every batch of pixels it sends to the
//! panel to [`record`]. `record` copies the pixels into a shadow framebuffer
//! in PSRAM and marks the changed columns of each row as dirty.
//!
//! Every 40 ms, the encoder task on CPU1 takes the dirty spans, reads the
//! pixels from the shadow framebuffer and writes them as Rect packets (see
//! [`hack_and_hike_core::screen`]) into the packet channel of the
//! [`serial`](super::serial) writer. It reads the pixels when it encodes
//! them, so a packet always carries the newest pixels. When CPU0 changes a
//! row while the encoder reads it, the row is dirty again and goes out once
//! more at the next tick.
//!
//! When the computer asks for a refresh, the encoder sends Hello and marks
//! the whole screen dirty.
//!
//! # Only while somebody watches
//!
//! The copy costs CPU0 time and PSRAM traffic for every pixel that goes to
//! the panel: about 6 ms for a camera preview of 184x240 pixels. An
//! application that draws many pixels and computes a lot can switch the
//! copy off while nobody watches ([`only_when_watched`]). Somebody watches
//! from start-up and from a refresh request, until the computer stops
//! reading the port for [`UNWATCHED_AFTER`]. At start-up because a page
//! that stays connected while the board restarts sends no refresh
//! request. The copy of the panel is out of date when the next viewer
//! comes, so the application draws everything again when [`refreshes`]
//! changes.

use core::{
    cell::{Cell, RefCell},
    sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
};

use allocator_api2::vec::Vec;
use critical_section::Mutex;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker, with_timeout};
use hack_and_hike_core::screen::{self, MAX_RECT_PIXELS, Rect};

use super::serial::{PacketSender, REFRESH, Slot};
use crate::{
    board::psram,
    capabilities::display::{HEIGHT, WIDTH},
};

/// Time between two encoder runs.
const TICK: Duration = Duration::from_millis(40);
/// A packet that waits this long for the serial port means that no program
/// on the computer reads the port: nobody watches.
const UNWATCHED_AFTER: Duration = Duration::from_secs(2);

/// Whether [`record`] copies pixels only while somebody watches.
static ONLY_WHEN_WATCHED: AtomicBool = AtomicBool::new(false);
/// Whether somebody watches: since start-up or a refresh request, the
/// computer has read the port.
static WATCHED: AtomicBool = AtomicBool::new(true);
/// The refresh requests so far.
static REFRESHES: AtomicU32 = AtomicU32::new(0);

/// Copy the panel for the screen feed only while somebody watches it
/// (`true`), or always (`false`, as after start-up).
///
/// An application that switches this on must draw its whole screen again
/// when [`mirror_refreshes`](crate::logging::mirror_refreshes) changes:
/// what it drew while nobody watched is not in the copy. For a [`Canvas`](crate::ui::Canvas), call
/// [`invalidate`](crate::ui::Canvas::invalidate) before the next `show`.
pub fn only_when_watched(enabled: bool) {
    ONLY_WHEN_WATCHED.store(enabled, Ordering::Relaxed);
}

/// How often a program on the computer has asked for the whole screen. It
/// asks when it starts to watch, so a change means a new viewer.
pub fn refreshes() -> u32 {
    REFRESHES.load(Ordering::Relaxed)
}

/// The dirty columns of one row, from `min` to `max`, both included. The
/// span is empty when `min > max`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Span {
    /// Leftmost dirty column.
    min: u16,
    /// Rightmost dirty column.
    max: u16,
}

impl Span {
    /// No dirty column.
    const EMPTY: Self = Self {
        min: u16::MAX,
        max: 0,
    };
    /// Every column dirty.
    const FULL: Self = Self {
        min: 0,
        max: WIDTH as u16 - 1,
    };

    /// Whether no column is dirty.
    const fn is_empty(self) -> bool {
        self.min > self.max
    }

    /// Widen the span to include the columns `first` to `last`.
    fn widen(&mut self, first: u16, last: u16) {
        self.min = self.min.min(first);
        self.max = self.max.max(last);
    }
}

/// The shadow framebuffer: [`WIDTH`] times [`HEIGHT`] RGB565 pixels in row
/// order. `None` until [`enable`] has run. CPU0 writes the pixels and CPU1
/// reads them. Relaxed atomic loads and stores are enough, because a row
/// that changes during a read is sent again (see the module documentation).
static SHADOW: Mutex<Cell<Option<&'static [AtomicU16]>>> = Mutex::new(Cell::new(None));
/// The dirty span of each row since the encoder's last run, [`HEIGHT`]
/// spans in PSRAM. `None` until [`enable`] has run. Like the shadow
/// framebuffer, it is not a static: statics take internal RAM from the CPU0
/// stack.
static DIRTY: Mutex<RefCell<Option<&'static mut [Span]>>> = Mutex::new(RefCell::new(None));

/// Run `f` on the dirty spans inside a critical section. Does nothing
/// before [`enable`].
fn with_dirty(f: impl FnOnce(&mut [Span])) {
    critical_section::with(|cs| {
        if let Some(spans) = DIRTY.borrow_ref_mut(cs).as_deref_mut() {
            f(spans);
        }
    });
}

/// The shadow framebuffer, or `None` before [`enable`].
fn shadow() -> Option<&'static [AtomicU16]> {
    critical_section::with(|cs| SHADOW.borrow(cs).get())
}

/// Allocate the shadow framebuffer (150 KiB) and the dirty spans in PSRAM. From now on,
/// [`record`] keeps a copy of the panel.
/// [`Board::init`](crate::Board::init) calls it after PSRAM is enabled and
/// before the display is set up.
///
/// # Panics
///
/// When PSRAM is not enabled yet, or has no room.
pub(crate) fn enable() {
    let mut pixels = Vec::with_capacity_in(WIDTH * HEIGHT, psram::heap());
    pixels.extend((0..WIDTH * HEIGHT).map(|_| AtomicU16::new(0)));
    let pixels: &'static [AtomicU16] = pixels.leak();
    let spans = psram::leaked_slice(HEIGHT, Span::EMPTY);
    critical_section::with(|cs| {
        SHADOW.borrow(cs).set(Some(pixels));
        *DIRTY.borrow_ref_mut(cs) = Some(spans);
    });
}

/// Copy one batch of pixels that the display transport is about to send,
/// and mark it dirty.
///
/// The batch is `rows` rows of `width` pixels, with its top-left pixel at
/// `(x, y)` in panel coordinates. `bytes` holds the pixels as big-endian
/// RGB565, row after row. Pixels outside the panel are ignored. Before
/// [`enable`], this function does nothing.
pub(crate) fn record(x: usize, y: usize, width: usize, rows: usize, bytes: &[u8]) {
    if ONLY_WHEN_WATCHED.load(Ordering::Relaxed) && !WATCHED.load(Ordering::Relaxed) {
        return;
    }
    let Some(shadow) = shadow() else {
        return;
    };
    if x >= WIDTH || y >= HEIGHT || width == 0 {
        return;
    }
    let visible_width = width.min(WIDTH - x);
    let visible_rows = rows.min(HEIGHT - y);

    // Store the pixels first and mark them dirty after. So the encoder
    // never clears a span before it can read the new pixels.
    for (row, line) in bytes.chunks_exact(width * 2).take(visible_rows).enumerate() {
        let start = (y + row) * WIDTH + x;
        for (pixel, pair) in shadow[start..start + visible_width]
            .iter()
            .zip(line.chunks_exact(2))
        {
            pixel.store(u16::from_be_bytes([pair[0], pair[1]]), Ordering::Relaxed);
        }
    }

    let first = x as u16;
    let last = (x + visible_width - 1) as u16;
    with_dirty(|spans| {
        for span in &mut spans[y..y + visible_rows] {
            span.widen(first, last);
        }
    });
}

/// Encode the screen changes into packets for the serial writer, forever.
#[embassy_executor::task]
pub(super) async fn encoder_task(mut packets: PacketSender) {
    let Some(shadow) = shadow() else {
        log::warn!("screen mirror: not enabled, no screen feed");
        return;
    };
    // The copy of the dirty spans that one run works on. It is in PSRAM,
    // not in the task.
    let snapshot = psram::leaked_slice(HEIGHT, Span::EMPTY);

    send_hello(&mut packets).await;
    let mut ticker = Ticker::every(TICK);
    loop {
        if let Either::Second(()) = select(ticker.next(), REFRESH.wait()).await {
            // Somebody watches from now on. Count the request after that,
            // so an application that sees the count draws into the copy.
            WATCHED.store(true, Ordering::Relaxed);
            REFRESHES.fetch_add(1, Ordering::Relaxed);
            send_hello(&mut packets).await;
            with_dirty(|spans| spans.fill(Span::FULL));
        }

        with_dirty(|spans| {
            for (copy, live) in snapshot.iter_mut().zip(spans.iter_mut()) {
                *copy = *live;
                *live = Span::EMPTY;
            }
        });
        send_dirty(&mut packets, shadow, snapshot).await;
    }
}

/// Wait for a free packet slot. When that takes [`UNWATCHED_AFTER`], the
/// computer does not read the port, so nobody watches any more.
async fn free_slot(packets: &mut PacketSender) -> &mut Slot {
    if with_timeout(UNWATCHED_AFTER, packets.send()).await.is_err() {
        WATCHED.store(false, Ordering::Relaxed);
    }
    packets.send().await
}

/// Send a Hello packet with the screen size.
async fn send_hello(packets: &mut PacketSender) {
    let slot = free_slot(packets).await;
    if screen::write_hello(WIDTH as u16, HEIGHT as u16, slot).is_some() {
        packets.send_done();
    }
}

/// Send the dirty rows of `spans` as Rect packets.
///
/// Neighbouring rows with the same span share a rectangle. A rectangle is
/// cut into packets of at most [`MAX_RECT_PIXELS`] pixels.
async fn send_dirty(packets: &mut PacketSender, shadow: &[AtomicU16], spans: &[Span]) {
    let mut row = 0;
    while row < spans.len() {
        let span = spans[row];
        if span.is_empty() {
            row += 1;
            continue;
        }
        let end = spans[row..]
            .iter()
            .position(|&other| other != span)
            .map_or(spans.len(), |offset| row + offset);

        let width = usize::from(span.max - span.min) + 1;
        let left = usize::from(span.min);
        let rows_per_packet = (MAX_RECT_PIXELS / width).max(1);
        for top in (row..end).step_by(rows_per_packet) {
            let rect = Rect {
                x: span.min,
                y: top as u16,
                w: width as u16,
                h: (end - top).min(rows_per_packet) as u16,
            };
            let pixel = |index: usize| {
                shadow[(top + index / width) * WIDTH + left + index % width].load(Ordering::Relaxed)
            };
            let slot = free_slot(packets).await;
            // A slot always fits `MAX_RECT_PIXELS` pixels, so this never
            // fails. If it did, the slot would stay free for the next packet.
            if screen::write_rect(rect, pixel, slot).is_some() {
                packets.send_done();
            }
        }
        row = end;
    }
}
