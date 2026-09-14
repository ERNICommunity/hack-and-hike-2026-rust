//! One-time setup of the ILI9342C panel controller.
//!
//! The `mipidsi` crate knows the controller's power-up sequence (sleep out,
//! pixel format, orientation, display on). It wants an `SpiDevice`, which
//! owns the bus for the whole display lifetime; we only borrow the bus for
//! setup and then take it back, so the pixel path in [`super::transport`] can
//! drive SPI DMA directly.

use embedded_hal::{
    delay::DelayNs as _,
    spi::{ErrorType as SpiErrorType, Operation, SpiBus, SpiDevice},
};
use embedded_hal_bus::spi::DeviceError;
use esp_hal::{Blocking, delay::Delay, gpio::Output, spi::master::SpiDma};
use mipidsi::options::{
    HorizontalRefreshOrder, Orientation, RefreshOrder, Rotation, VerticalRefreshOrder,
};

use crate::platform;

type DisplaySpiDma = SpiDma<'static, Blocking>;

/// What the transport needs back after setup.
pub(super) struct Initialized {
    /// The SPI DMA driver, with the command buffers still attached.
    pub(super) spi: DisplaySpiDma,
    /// Chip select.
    pub(super) cs: Output<'static>,
    /// Data/command select.
    pub(super) dc: Output<'static>,
}

/// An `SpiDevice` that owns its bus and chip select only until `release`.
///
/// `embedded-hal-bus` has a similar `ExclusiveDevice`, but it cannot give the
/// bus back, which the transport needs.
struct OwnedSpiDevice<BUS, CS> {
    bus: BUS,
    cs: CS,
}

impl<BUS, CS> OwnedSpiDevice<BUS, CS>
where
    CS: embedded_hal::digital::OutputPin,
{
    /// Take `bus` and `cs`, with chip select released (high).
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
    /// Run `operations` with chip select low, stopping at the first error.
    /// Chip select goes high again in every case.
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

/// Run the controller's power-up sequence for this board's panel and return
/// the bus and control pins.
pub(super) fn initialize(
    dma_bus: DisplaySpiDma,
    cs: Output<'static>,
    dc: Output<'static>,
    mut delay: Delay,
) -> Initialized {
    let spi_device = OwnedSpiDevice::new(dma_bus, cs).expect("Failed to initialize LCD SPI device");
    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

    // The board is mounted 180 degrees, so logical top-to-bottom/left-to-right
    // GRAM writes travel physically bottom-to-top/right-to-left. Match the
    // controller refresh direction to that same physical direction.
    let (orientation, refresh_order) = if platform::DISPLAY_ROTATED_180 {
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
