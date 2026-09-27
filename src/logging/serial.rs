//! The USB side of the log: one serial port for log text and screen packets.
//!
//! The board's USB-C socket is wired to the ESP32-S3's USB Serial/JTAG
//! peripheral. The computer sees it as a serial port. Two kinds of data
//! share it:
//!
//! - log lines, as plain UTF-8 text, queued in the text queue by
//!   [`push_text`];
//! - screen packets from the [`mirror`](super::mirror), queued in a channel
//!   of packet slots.
//!
//! The writer task sends both. Text goes first, so a busy screen never holds
//! back the log. A packet is always sent whole, so text never lands inside a
//! packet.
//!
//! When no program on the computer reads the port, the writer waits. Then
//! the text queue fills up, and new lines are dropped and counted. The
//! screen encoder waits for a free slot. Both continue when a program opens
//! the port.

use core::{
    cell::Cell,
    fmt::Write as _,
    sync::atomic::{AtomicBool, Ordering},
};

use arrayvec::ArrayString;
use critical_section::Mutex;
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    pipe::Pipe,
    signal::Signal,
    zerocopy_channel::{Channel, Receiver, Sender},
};
use embedded_io_async::{Read, Write};
use esp_hal::{
    Async,
    usb::usb_serial_jtag::{UsbSerialJtagRx, UsbSerialJtagTx},
};
use hack_and_hike_core::screen::{DELIMITER, SLOT_BYTES};
use static_cell::StaticCell;

use crate::board::psram;

/// Size of the text queue in internal RAM. It holds the boot log until a
/// computer reads the port.
const TEXT_BYTES: usize = 8192;
/// The text queue. Both cores write to it.
pub(super) type TextQueue = Pipe<CriticalSectionRawMutex, TEXT_BYTES>;
/// Bytes the USB peripheral sends in one USB packet. The writer checks for a
/// panic before each such chunk.
const CHUNK_BYTES: usize = 64;
/// Number of packet slots. One is filled while the other is sent.
const SLOTS: usize = 2;

/// One packet slot: a whole screen packet, delimiters included. The bytes
/// after the closing delimiter are left over from older packets.
pub(super) type Slot = [u8; SLOT_BYTES];
/// The encoder's end of the packet channel.
pub(super) type PacketSender = Sender<'static, CriticalSectionRawMutex, Slot>;
/// The writer's end of the packet channel.
pub(super) type PacketReceiver = Receiver<'static, CriticalSectionRawMutex, Slot>;

/// Log text waiting for the writer. `None` until [`create_text_queue`].
///
/// The queue is on the internal heap, not in a static. On this chip, the
/// CPU0 stack gets the internal RAM that the statics leave free, so an 8 KiB
/// static would make the stack of every application 8 KiB smaller.
static TEXT: Mutex<Cell<Option<&'static TextQueue>>> = Mutex::new(Cell::new(None));
/// Log lines dropped because the text queue was full, since the last line
/// that fitted.
static DROPPED: Mutex<Cell<u32>> = Mutex::new(Cell::new(0));
/// The packet channel. Its two slots are in PSRAM, never on a stack.
static PACKETS: StaticCell<Channel<'static, CriticalSectionRawMutex, Slot>> = StaticCell::new();
/// Set by the panic hook. The writer stops before its next chunk, so the
/// panic handler has the port to itself.
pub(super) static PANICKED: AtomicBool = AtomicBool::new(false);
/// Signalled when the computer asks for the whole screen again.
pub(super) static REFRESH: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Queue `text` for the serial port, all of it or nothing. A `0x00` byte
/// becomes a space, because `0x00` marks screen packets.
///
/// When the queue is too full, the text is dropped and counted. The next
/// text that fits is preceded by a warning with the count.
pub(super) fn push_text(text: &str) {
    critical_section::with(|cs| {
        let Some(queue) = TEXT.borrow(cs).get() else {
            return;
        };
        let dropped = DROPPED.borrow(cs);
        let mut warning = ArrayString::<48>::new();
        if dropped.get() > 0 {
            let _ = writeln!(
                warning,
                "[WARN] serial: {} log lines dropped",
                dropped.get()
            );
        }
        if queue.free_capacity() < warning.len() + text.len() {
            dropped.set(dropped.get().saturating_add(1));
            return;
        }
        dropped.set(0);
        // The critical section keeps the other core out, so every write
        // below fits in the space checked above.
        let _ = queue.try_write(warning.as_bytes());
        for (index, part) in text.split('\0').enumerate() {
            if index > 0 {
                let _ = queue.try_write(b" ");
            }
            let _ = queue.try_write(part.as_bytes());
        }
    });
}

/// Allocate the text queue on the internal heap. Until then, [`push_text`]
/// drops all text. Call it once, when the heap exists and the stack is still
/// shallow: the queue is built on the stack before it moves to the heap.
pub(super) fn create_text_queue() {
    let queue: &'static TextQueue = alloc::boxed::Box::leak(alloc::boxed::Box::new(Pipe::new()));
    critical_section::with(|cs| TEXT.borrow(cs).set(Some(queue)));
}

/// The text queue, or `None` before [`create_text_queue`].
pub(super) fn text_queue() -> Option<&'static TextQueue> {
    critical_section::with(|cs| TEXT.borrow(cs).get())
}

/// Create the packet channel in PSRAM and return its two ends.
///
/// # Panics
///
/// When it is called a second time, or PSRAM has no room.
pub(super) fn packet_channel() -> (PacketSender, PacketReceiver) {
    let slots = psram::leaked_slice(SLOTS, [0; SLOT_BYTES]);
    PACKETS.init(Channel::new(slots)).split()
}

/// Send text and packets to the computer, text first.
#[embassy_executor::task]
pub(super) async fn writer_task(
    mut tx: UsbSerialJtagTx<'static, Async>,
    queue: &'static TextQueue,
    mut packets: PacketReceiver,
) {
    let mut text = [0; CHUNK_BYTES];
    loop {
        // `select` polls the text first, so text wins when both are ready.
        match select(queue.read(&mut text), packets.receive()).await {
            Either::First(len) => write(&mut tx, &text[..len]).await,
            Either::Second(slot) => {
                let len = packet_len(slot);
                write(&mut tx, &slot[..len]).await;
                packets.receive_done();
            }
        }
    }
}

/// Length of the packet in `slot`: up to and including the closing
/// delimiter.
fn packet_len(slot: &Slot) -> usize {
    slot.iter()
        .skip(1)
        .position(|&byte| byte == DELIMITER)
        .map_or(slot.len(), |at| at + 2)
}

/// Write `bytes` in chunks of [`CHUNK_BYTES`]. Park forever after a panic.
async fn write(tx: &mut UsbSerialJtagTx<'static, Async>, bytes: &[u8]) {
    for chunk in bytes.chunks(CHUNK_BYTES) {
        park_after_panic().await;
        // The driver's own blocking `write` has the same name.
        let _ = Write::write(tx, chunk).await;
    }
    // A last chunk of exactly 64 bytes does not end the USB transfer. The
    // computer would hold it back until more data comes. An empty USB
    // packet ends the transfer.
    if bytes.len().is_multiple_of(CHUNK_BYTES) && !bytes.is_empty() {
        park_after_panic().await;
        let _ = Write::flush(tx).await;
    }
}

/// Never return once the panic hook has run.
async fn park_after_panic() {
    if PANICKED.load(Ordering::Acquire) {
        core::future::pending::<()>().await;
    }
}

/// Wait for bytes from the computer. Each read is a refresh request.
#[embassy_executor::task]
pub(super) async fn reader_task(mut rx: UsbSerialJtagRx<'static, Async>) {
    let mut request = [0; 8];
    loop {
        if Read::read(&mut rx, &mut request).await.is_ok() {
            REFRESH.signal(());
        }
    }
}
