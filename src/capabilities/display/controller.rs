//! One-time setup of the ILI9342C panel controller.
//!
//! The ILI9342C is the chip on the LCD panel that receives commands and
//! pixels over SPI. The `mipidsi` crate knows its power-up sequence: leave
//! sleep mode, set the pixel format, set the orientation, turn the display
//! on. `mipidsi` needs an `SpiDevice`, which normally owns the bus for as
//! long as the display exists. This module gives the bus to `mipidsi` only
//! for the setup and then takes it back. After that, the pixel path in
//! [`super::transport`] uses SPI with DMA directly.

use embedded_hal::{
    delay::DelayNs as _,
    spi::{ErrorType as SpiErrorType, Operation, SpiBus, SpiDevice},
};
use embedded_hal_bus::spi::DeviceError;
use esp_hal::{Blocking, delay::Delay, gpio::Output, spi::master::SpiDma};
use mipidsi::options::{
    HorizontalRefreshOrder, Orientation, RefreshOrder, Rotation, VerticalRefreshOrder,
};

use crate::board;

/// The blocking SPI driver with DMA. The setup and the pixel path both use
/// it.
type DisplaySpiDma = SpiDma<'static, Blocking>;

/// What the transport gets back after the setup.
pub(super) struct Initialized {
    /// The SPI DMA driver. The small DMA buffers for the setup commands are
    /// still attached to it.
    pub(super) spi: DisplaySpiDma,
    /// Chip select (CS), active low.
    pub(super) cs: Output<'static>,
    /// Data/command select (DC): low for a command byte, high for data.
    pub(super) dc: Output<'static>,
}

/// An `SpiDevice` that owns its bus and chip select pin until
/// [`OwnedSpiDevice::release`] gives them back.
///
/// `embedded-hal-bus` has a similar type, `ExclusiveDevice`. But it cannot
/// give the bus back, and the transport needs the bus after the setup.
struct OwnedSpiDevice<BUS, CS> {
    /// The SPI bus. Only this device uses it until `release`.
    bus: BUS,
    /// Chip select pin: low during each transaction, high otherwise.
    cs: CS,
}

impl<BUS, CS> OwnedSpiDevice<BUS, CS>
where
    CS: embedded_hal::digital::OutputPin,
{
    /// Take `bus` and `cs`, and set chip select high (not selected).
    ///
    /// # Errors
    ///
    /// The pin error when chip select cannot be set.
    fn new(bus: BUS, mut cs: CS) -> Result<Self, CS::Error> {
        cs.set_high()?;
        Ok(Self { bus, cs })
    }

    /// Give the bus and chip select back.
    fn release(self) -> (BUS, CS) {
        (self.bus, self.cs)
    }
}

impl<BUS, CS> SpiErrorType for OwnedSpiDevice<BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: embedded_hal::digital::OutputPin,
{
    type Error = DeviceError<BUS::Error, CS::Error>;
}

impl<BUS, CS> SpiDevice<u8> for OwnedSpiDevice<BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: embedded_hal::digital::OutputPin,
{
    /// Run `operations` with chip select low. Stop at the first error.
    ///
    /// After all operations succeed, flush the bus, so the last bytes have
    /// left it. Once chip select is low, it goes high again, also after an
    /// error. When chip select cannot be set low, nothing runs.
    ///
    /// # Errors
    ///
    /// The first SPI or pin error. An SPI error has priority over an error
    /// when chip select goes high.
    fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Self::Error> {
        self.cs.set_low().map_err(DeviceError::Cs)?;

        let mut result = Ok(());
        let mut delay = Delay::new();

        for operation in operations {
            let operation_result = match operation {
                Operation::Read(words) => self.bus.read(words),
                Operation::Write(words) => self.bus.write(words),
                Operation::Transfer(read, write) => self.bus.transfer(read, write),
                Operation::TransferInPlace(words) => self.bus.transfer_in_place(words),
                Operation::DelayNs(ns) => {
                    delay.delay_ns(*ns);
                    Ok(())
                }
            };

            if let Err(err) = operation_result {
                result = Err(DeviceError::Spi(err));
                break;
            }
        }

        if result.is_ok()
            && let Err(err) = self.bus.flush()
        {
            result = Err(DeviceError::Spi(err));
        }

        let cs_result = self.cs.set_high().map_err(DeviceError::Cs);
        match result {
            Err(err) => Err(err),
            Ok(()) => cs_result,
        }
    }
}

/// Run the controller's power-up sequence for this board's panel. Return the
/// bus and the control pins.
///
/// # Panics
///
/// When chip select cannot be set, or the setup over SPI fails.
pub(super) fn initialize(
    dma_bus: DisplaySpiDma,
    cs: Output<'static>,
    dc: Output<'static>,
    mut delay: Delay,
) -> Initialized {
    let spi_device = OwnedSpiDevice::new(dma_bus, cs).expect("Failed to initialize LCD SPI device");
    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

    // On this board, the panel is mounted at 180 degrees
    // (`board::DISPLAY_ROTATED_180`). GRAM (graphics RAM) is the controller's
    // memory for the image. The firmware writes GRAM from top to bottom and
    // from left to right, in its own coordinates. On the mounted panel, these
    // writes go from bottom to top and from right to left. The refresh order
    // is the order in which the controller updates the screen from GRAM.
    // This code sets it to the same physical direction as the writes.
    let (orientation, refresh_order) = if board::DISPLAY_ROTATED_180 {
        (
            Orientation::new().rotate(Rotation::Deg180),
            RefreshOrder {
                vertical: VerticalRefreshOrder::BottomToTop,
                horizontal: HorizontalRefreshOrder::RightToLeft,
            },
        )
    } else {
        (
            Orientation::new(),
            RefreshOrder {
                vertical: VerticalRefreshOrder::TopToBottom,
                horizontal: HorizontalRefreshOrder::LeftToRight,
            },
        )
    };

    let display = mipidsi::Builder::new(mipidsi::models::ILI9342CRgb565, di)
        .color_order(mipidsi::options::ColorOrder::Bgr)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .orientation(orientation)
        .refresh_order(refresh_order)
        .init(&mut delay)
        .expect("ILI9342C initialization over SPI failed");

    let (di, _model, _reset) = display.release();
    let (spi_device, dc) = di.release();
    let (spi, cs) = spi_device.release();

    Initialized { spi, cs, dc }
}
