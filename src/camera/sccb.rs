//! Startup-only software SCCB transport for the onboard GC0308.
//!
//! CoreS3 routes camera SCCB and the board system I2C bus to the same GPIO12/
//! GPIO11 pair. M5Stack's camera implementation releases its shared I2C owner
//! before camera initialization. We mirror that ownership boundary here and use
//! a tiny software SCCB master while the persistent runtime I2C driver does not
//! exist. Runtime camera capture never uses this transport.

use core::convert::Infallible;

use embedded_hal::i2c::{ErrorType, I2c, Operation, SevenBitAddress};
use esp_hal::{
    delay::Delay,
    gpio::{DriveMode, Flex, InputConfig, Level, OutputConfig, Pull},
    peripherals::{GPIO11, GPIO12},
};

// Nominal 100 kHz SCCB. GPIO and function-call overhead make the real clock a
// little slower, which is harmless for sensor startup and improves margin.
const HALF_PERIOD_US: u32 = 5;

pub(super) struct Sccb<'d> {
    sda: Flex<'d>,
    scl: Flex<'d>,
    delay: Delay,
    nack_count: u16,
}

impl<'d> Sccb<'d> {
    pub(super) fn new(sda: GPIO12<'d>, scl: GPIO11<'d>) -> Self {
        let mut sda = Flex::new(sda);
        let mut scl = Flex::new(scl);
        let output = OutputConfig::default().with_drive_mode(DriveMode::OpenDrain);
        let input = InputConfig::default().with_pull(Pull::Up);

        // Set the output latch high before enabling open-drain output so taking
        // ownership cannot create a spurious low pulse. Input stays enabled so
        // ACK/data can be sampled while the line is released.
        sda.apply_output_config(&output);
        sda.apply_input_config(&input);
        sda.set_level(Level::High);
        sda.set_input_enable(true);
        sda.set_output_enable(true);

        scl.apply_output_config(&output);
        scl.apply_input_config(&input);
        scl.set_level(Level::High);
        scl.set_input_enable(true);
        scl.set_output_enable(true);

        Self {
            sda,
            scl,
            delay: Delay::new(),
            nack_count: 0,
        }
    }

    /// Recover a half-finished transaction left by an earlier peripheral owner,
    /// then put SCCB into a known STOP/idle state before sensor programming.
    pub(super) fn recover_bus(&mut self) {
        self.release_sda();
        self.release_scl();
        self.half_period();

        for _ in 0..9 {
            self.drive_scl_low();
            self.half_period();
            self.release_scl();
            self.half_period();
        }

        self.stop();
    }

    pub(super) fn nack_count(&self) -> u16 {
        self.nack_count
    }

    fn half_period(&self) {
        self.delay.delay_micros(HALF_PERIOD_US);
    }

    fn drive_sda_low(&mut self) {
        self.sda.set_level(Level::Low);
    }

    fn release_sda(&mut self) {
        self.sda.set_level(Level::High);
    }

    fn drive_scl_low(&mut self) {
        self.scl.set_level(Level::Low);
    }

    fn release_scl(&mut self) {
        self.scl.set_level(Level::High);
    }

    fn start(&mut self) {
        self.release_sda();
        self.release_scl();
        self.half_period();
        self.drive_sda_low();
        self.half_period();
        self.drive_scl_low();
        self.half_period();
    }

    fn stop(&mut self) {
        self.drive_sda_low();
        self.half_period();
        self.release_scl();
        self.half_period();
        self.release_sda();
        self.half_period();
    }

    fn write_byte(&mut self, value: u8) {
        for bit in (0..8).rev() {
            if value & (1 << bit) == 0 {
                self.drive_sda_low();
            } else {
                self.release_sda();
            }
            self.half_period();
            self.release_scl();
            self.half_period();
            self.drive_scl_low();
            self.half_period();
        }

        // SCCB devices are not uniformly strict about ACK behavior. Sample and
        // count it for diagnostics, but do not abort the register program. PID
        // validation at the end is the authoritative bring-up check.
        self.release_sda();
        self.half_period();
        self.release_scl();
        self.half_period();
        if self.sda.is_high() {
            self.nack_count = self.nack_count.saturating_add(1);
        }
        self.drive_scl_low();
        self.half_period();
    }

    fn read_byte(&mut self, acknowledge: bool) -> u8 {
        self.release_sda();
        let mut value = 0u8;

        for _ in 0..8 {
            self.half_period();
            self.release_scl();
            self.half_period();
            value = (value << 1) | u8::from(self.sda.is_high());
            self.drive_scl_low();
            self.half_period();
        }

        if acknowledge {
            self.drive_sda_low();
        } else {
            self.release_sda();
        }
        self.half_period();
        self.release_scl();
        self.half_period();
        self.drive_scl_low();
        self.release_sda();
        self.half_period();

        value
    }

    fn write_phase(&mut self, address: SevenBitAddress, bytes: &[u8]) {
        self.start();
        self.write_byte(address << 1);
        for &byte in bytes {
            self.write_byte(byte);
        }
        self.stop();
    }

    fn read_phase(&mut self, address: SevenBitAddress, bytes: &mut [u8]) {
        self.start();
        self.write_byte((address << 1) | 1);
        let last = bytes.len().saturating_sub(1);
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.read_byte(index != last);
        }
        self.stop();
    }
}

impl ErrorType for Sccb<'_> {
    type Error = Infallible;
}

impl I2c<SevenBitAddress> for Sccb<'_> {
    fn read(&mut self, address: SevenBitAddress, read: &mut [u8]) -> Result<(), Self::Error> {
        self.read_phase(address, read);
        Ok(())
    }

    fn write(&mut self, address: SevenBitAddress, write: &[u8]) -> Result<(), Self::Error> {
        self.write_phase(address, write);
        Ok(())
    }

    fn write_read(
        &mut self,
        address: SevenBitAddress,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        // OmniVision SCCB read is a two-phase operation with STOP between the
        // sub-address write and data read, matching Espressif's camera driver.
        self.write_phase(address, write);
        self.read_phase(address, read);
        Ok(())
    }

    fn transaction(
        &mut self,
        address: SevenBitAddress,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        // This transport is intentionally camera-specific. Keep each operation
        // as an SCCB phase rather than exposing generic repeated-start behavior.
        for operation in operations {
            match operation {
                Operation::Read(bytes) => self.read_phase(address, bytes),
                Operation::Write(bytes) => self.write_phase(address, bytes),
            }
        }
        Ok(())
    }
}
