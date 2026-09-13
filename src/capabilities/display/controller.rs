//! One-time ILI9342C controller initialization.
//!
//! Runtime pixel transport lives in `transport`; this module exists only to run
//! the `mipidsi` setup sequence and return the owned SPI/DMA resources.

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

pub(super) struct Initialized {
    pub(super) spi: DisplaySpiDma,
    pub(super) cs: Output<'static>,
    pub(super) dc: Output<'static>,
}

/// Small owned `SpiDevice` adapter used only during `mipidsi` initialization.
/// It can be deconstructed afterwards so steady-state rendering can use raw
/// pipelined SPI-DMA.
struct OwnedSpiDevice<BUS, CS> {
    bus: BUS,
    cs: CS,
}

impl<BUS, CS> OwnedSpiDevice<BUS, CS>
where
    CS: embedded_hal::digital::OutputPin,
{
    fn new(bus: BUS, mut cs: CS) -> Result<Self, CS::Error> {
        cs.set_high()?;
        Ok(Self { bus, cs })
    }

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
        .unwrap();

    let (di, _model, _reset) = display.release();
    let (spi_device, dc) = di.release();
    let (spi, cs) = spi_device.release();

    Initialized { spi, cs, dc }
}
