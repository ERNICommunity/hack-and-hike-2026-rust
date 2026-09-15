//! Register access shared by the I2C chips on the board.
//!
//! Most chips on the bus have 8-bit registers at 8-bit register addresses.
//! To read, write the register address, then read the value. To write, send
//! the register address and the value together. The two small types here
//! connect a bus with one chip address. So driver code looks like a register
//! map:
//!
//! ```ignore
//! let mut pmic = Registers::new(&mut i2c, 0x34);
//! pmic.update_bits(OUTPUT_ENABLE_REGISTER, DLDO1_ENABLE, DLDO1_ENABLE)?;
//! ```

/// One I2C chip with 8-bit registers, borrowed for a few transfers.
pub(crate) struct Registers<'a, I2C> {
    /// The bus, borrowed exclusively while this value lives.
    i2c: &'a mut I2C,
    /// 7-bit I2C address of the chip.
    address: u8,
}

impl<'a, I2C: embedded_hal::i2c::I2c> Registers<'a, I2C> {
    /// The chip at 7-bit `address` on `i2c`.
    pub(crate) fn new(i2c: &'a mut I2C, address: u8) -> Self {
        Self { i2c, address }
    }

    /// Read one register.
    pub(crate) fn read(&mut self, register: u8) -> Result<u8, I2C::Error> {
        let mut value = [0u8; 1];
        self.i2c.write_read(self.address, &[register], &mut value)?;
        Ok(value[0])
    }

    /// Write one register.
    pub(crate) fn write(&mut self, register: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(self.address, &[register, value])
    }

    /// Write several `(register, value)` pairs in order.
    pub(crate) fn write_all(&mut self, values: &[(u8, u8)]) -> Result<(), I2C::Error> {
        values
            .iter()
            .try_for_each(|&(register, value)| self.write(register, value))
    }

    /// Set the bits selected by `mask` to the bits of `value`, and keep the
    /// other bits. Reads the register, changes those bits and writes it back.
    pub(crate) fn update_bits(
        &mut self,
        register: u8,
        mask: u8,
        value: u8,
    ) -> Result<(), I2C::Error> {
        let current = self.read(register)?;
        self.write(register, (current & !mask) | (value & mask))
    }
}

/// The async version of [`Registers`], for CPU1 tasks on the shared bus.
pub(crate) struct AsyncRegisters<'a, I2C> {
    /// The bus, borrowed exclusively while this value lives.
    i2c: &'a mut I2C,
    /// 7-bit I2C address of the chip.
    address: u8,
}

impl<'a, I2C: embedded_hal_async::i2c::I2c> AsyncRegisters<'a, I2C> {
    /// The chip at 7-bit `address` on `i2c`.
    pub(crate) fn new(i2c: &'a mut I2C, address: u8) -> Self {
        Self { i2c, address }
    }

    /// Write one register.
    pub(crate) async fn write(&mut self, register: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(self.address, &[register, value]).await
    }

    /// Set the bits selected by `mask` to the bits of `value`, and keep the
    /// other bits. Reads the register, changes those bits and writes it back.
    pub(crate) async fn update_bits(
        &mut self,
        register: u8,
        mask: u8,
        value: u8,
    ) -> Result<(), I2C::Error> {
        let mut current = [0u8; 1];
        self.i2c
            .write_read(self.address, &[register], &mut current)
            .await?;
        self.write(register, (current[0] & !mask) | (value & mask))
            .await
    }
}
