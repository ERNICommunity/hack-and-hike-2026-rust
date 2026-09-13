//! Register access shared by the I2C chips on the board.

/// One I2C chip with 8-bit registers, borrowed for a few transfers.
pub(crate) struct Registers<'a, I2C> {
    i2c: &'a mut I2C,
    address: u8,
}

impl<'a, I2C: embedded_hal::i2c::I2c> Registers<'a, I2C> {
    pub(crate) fn new(i2c: &'a mut I2C, address: u8) -> Self {
        Self { i2c, address }
    }

    pub(crate) fn read(&mut self, register: u8) -> Result<u8, I2C::Error> {
        let mut value = [0u8; 1];
        self.i2c.write_read(self.address, &[register], &mut value)?;
        Ok(value[0])
    }

    pub(crate) fn write(&mut self, register: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(self.address, &[register, value])
    }

    /// Write several `(register, value)` pairs in order.
    pub(crate) fn write_all(&mut self, values: &[(u8, u8)]) -> Result<(), I2C::Error> {
        values
            .iter()
            .try_for_each(|&(register, value)| self.write(register, value))
    }

    /// Change only the bits selected by `mask` to those of `value`.
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

/// The async twin of [`Registers`], for CPU1 tasks on the shared bus.
pub(crate) struct AsyncRegisters<'a, I2C> {
    i2c: &'a mut I2C,
    address: u8,
}

impl<'a, I2C: embedded_hal_async::i2c::I2c> AsyncRegisters<'a, I2C> {
    pub(crate) fn new(i2c: &'a mut I2C, address: u8) -> Self {
        Self { i2c, address }
    }

    pub(crate) async fn write(&mut self, register: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(self.address, &[register, value]).await
    }

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
