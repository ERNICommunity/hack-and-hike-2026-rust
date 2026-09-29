//! Data that survives a restart, in the flash chip.
//!
//! The storage keeps one record: a block of bytes under a name. Saving
//! replaces the record, whichever application saved it. Loading returns the
//! record only when it has the same name, so an application never reads the
//! data of another application as its own.
//!
//! ```ignore
//! if let Some(storage) = storage.as_mut() {
//!     if let Err(error) = storage.save("my_app/settings/v1", &bytes) {
//!         log::warn!("not saved: {error:?}");
//!     }
//!     if let Ok(saved) = storage.load("my_app/settings/v1", &mut buffer) {
//!         // `saved` is the start of `buffer`.
//!     }
//! }
//! ```
//!
//! The record keeps through a restart, a power-off and a new flash of the
//! `firmware.bin` of `cargo dist`. The format of the record is in
//! `hack_and_hike_core::storage`.
//!
//! Where the record lies: the flash chip has 16 MiB, but the image header
//! of `firmware.bin` says 4 MiB, and the ROM's flash functions refuse
//! addresses above the size in that header. So the record lies in the last
//! 64 KiB of the first 4 MiB. That is the end of the application's
//! partition, which the application (1 to 2.5 MiB) does not use.
//! `cargo dist` does not pad `firmware.bin` to 4 MiB (`--skip-padding`),
//! so flashing it keeps the record. Face ID keeps its enrollments in the
//! 128 KiB below the record.
//!
//! Saving is slow and stops other work, so save only when the data changed:
//!
//! - Each 4 KiB sector must be erased before it is written. Erasing and
//!   writing 32 KiB usually takes about half a second, and up to a few
//!   seconds on a slow flash chip.
//! - While the flash chip works, the CPUs cannot read the flash or PSRAM.
//!   So `save` stops CPU1 (touch, IMU, audio, radio) during each step, and
//!   interrupts on CPU0 wait. Pause the camera before saving: its buffer
//!   overflows otherwise.
//!
//! The storage is optional, like the camera. [`Board::init`](crate::Board::init)
//! returns `None` for it when the flash chip is smaller than 4 MiB.

use esp_hal::peripherals::FLASH;
use esp_storage::{FlashStorage, FlashStorageError};
use hack_and_hike_core::storage::{Crc32, HEADER_LEN, Header, kind};
use log::{info, warn};

/// Bytes of flash reserved for the record: 64 KiB, 16 sectors.
const REGION_BYTES: u32 = 64 * 1024;
/// The flash size in the image header of `firmware.bin`. The ROM's flash
/// functions refuse addresses above it, so the record must lie below it.
const IMAGE_BYTES: u32 = 4 * 1024 * 1024;
/// The largest record: the region without the header.
pub const MAX_LEN: usize = REGION_BYTES as usize - HEADER_LEN;
/// Bytes of one flash sector, the smallest part that can be erased.
const SECTOR: usize = FlashStorage::SECTOR_SIZE as usize;
/// Bytes of one flash word. Flash reads and writes need whole words.
const WORD: usize = FlashStorage::WORD_SIZE as usize;

/// Why a record could not be loaded or saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// Nothing is saved, or it was saved in an older format.
    Empty,
    /// The saved record has another name: another application, or an
    /// older version of the same data.
    OtherRecord,
    /// The data does not fit: longer than [`MAX_LEN`] when saving, or longer
    /// than the buffer when loading.
    TooLarge,
    /// The saved record is damaged: its length is impossible, or its data
    /// does not match its checksum.
    Corrupt,
    /// The flash chip reported an error. The log has the details.
    Flash,
}

/// Application handle for the storage; see the [module docs](self).
pub struct Storage {
    /// The flash chip.
    flash: FlashStorage<'static>,
    /// Flash address of the header; the data follows it.
    base: u32,
}

/// A sector of internal RAM, aligned to a flash word.
///
/// The flash chip must read from and write to internal RAM: while it works,
/// PSRAM cannot be read. So all data goes through this buffer.
#[repr(C, align(4))]
struct Sector([u8; SECTOR]);

/// Reserve the record's region at the end of the first 4 MiB.
///
/// Return `None` and log a warning when the flash chip is smaller.
pub(crate) fn init(flash: FLASH<'static>) -> Option<Storage> {
    // While the flash chip writes, CPU1 must not read the flash or PSRAM.
    // `multicore_auto_park` stops CPU1 for each write and starts it again.
    let flash = FlashStorage::new(flash).multicore_auto_park();
    let capacity = u32::try_from(flash.capacity()).unwrap_or(u32::MAX);
    if capacity < IMAGE_BYTES {
        warn!("Storage disabled: flash has only {} KiB", capacity / 1024);
        return None;
    }
    let base = IMAGE_BYTES - REGION_BYTES;
    info!(
        "Storage ready: {} KiB flash, record at 0x{:06x}",
        capacity / 1024,
        base
    );
    Some(Storage { flash, base })
}

impl Storage {
    /// Load the record named `name` into `buffer` and return its bytes: the
    /// start of `buffer`.
    ///
    /// # Errors
    ///
    /// [`StorageError::Empty`] when nothing is saved,
    /// [`StorageError::OtherRecord`] when the record has another name,
    /// [`StorageError::TooLarge`] when it does not fit into `buffer`,
    /// [`StorageError::Corrupt`] when its length or its checksum is wrong,
    /// and [`StorageError::Flash`] when the flash chip reports an error.
    pub fn load<'b>(&mut self, name: &str, buffer: &'b mut [u8]) -> Result<&'b [u8], StorageError> {
        let mut sector = Sector([0; SECTOR]);
        self.flash
            .read_nor(self.base, &mut sector.0[..HEADER_LEN])
            .map_err(flash_error)?;
        let header_bytes: &[u8; HEADER_LEN] = (&sector.0[..HEADER_LEN])
            .try_into()
            .expect("the slice has HEADER_LEN bytes");
        let header = Header::parse(header_bytes).ok_or(StorageError::Empty)?;
        if header.kind != kind(name) {
            return Err(StorageError::OtherRecord);
        }
        let length = header.length as usize;
        if length > MAX_LEN {
            // No save writes such a header.
            return Err(StorageError::Corrupt);
        }
        if length > buffer.len() {
            return Err(StorageError::TooLarge);
        }

        // Read sector by sector into internal RAM, then copy into `buffer`,
        // which may be in PSRAM. Reads need whole words.
        let data = &mut buffer[..length];
        let mut offset = self.base + HEADER_LEN as u32;
        let mut crc = Crc32::new();
        for chunk in data.chunks_mut(SECTOR) {
            let words = chunk.len().next_multiple_of(WORD);
            self.flash
                .read_nor(offset, &mut sector.0[..words])
                .map_err(flash_error)?;
            chunk.copy_from_slice(&sector.0[..chunk.len()]);
            crc.update(chunk);
            offset += SECTOR as u32;
        }
        if crc.finish() != header.checksum {
            return Err(StorageError::Corrupt);
        }
        Ok(data)
    }

    /// Save `data` as the record named `name`. It replaces the saved record,
    /// whatever its name. See the [module docs](self): this takes about half
    /// a second and stops CPU1.
    ///
    /// # Errors
    ///
    /// [`StorageError::TooLarge`] when `data` is longer than [`MAX_LEN`],
    /// and [`StorageError::Flash`] when the flash chip reports an error.
    /// After a flash error, nothing is saved.
    pub fn save(&mut self, name: &str, data: &[u8]) -> Result<(), StorageError> {
        if data.len() > MAX_LEN {
            return Err(StorageError::TooLarge);
        }
        let header = Header::new(name, data);
        let used = (HEADER_LEN + data.len()).next_multiple_of(SECTOR) as u32;
        self.flash
            .erase(self.base, self.base + used)
            .map_err(flash_error)?;

        // The data first, sector by sector through internal RAM. The end of
        // the last word is padded with erased bytes.
        let mut sector = Sector([0xFF; SECTOR]);
        let mut offset = self.base + HEADER_LEN as u32;
        for chunk in data.chunks(SECTOR) {
            let words = chunk.len().next_multiple_of(WORD);
            sector.0[..chunk.len()].copy_from_slice(chunk);
            sector.0[chunk.len()..words].fill(0xFF);
            self.flash
                .write_nor(offset, &sector.0[..words])
                .map_err(flash_error)?;
            offset += SECTOR as u32;
        }

        // The header last: until it is written, the record is not valid.
        sector.0[..HEADER_LEN].copy_from_slice(&header.to_bytes());
        self.flash
            .write_nor(self.base, &sector.0[..HEADER_LEN])
            .map_err(flash_error)?;
        info!("Saved {} bytes as {}", data.len(), name);
        Ok(())
    }
}

/// Log a flash error and turn it into [`StorageError::Flash`], so the
/// error type of the flash driver stays inside the capability.
fn flash_error(error: FlashStorageError) -> StorageError {
    warn!("Flash error: {error:?}");
    StorageError::Flash
}
