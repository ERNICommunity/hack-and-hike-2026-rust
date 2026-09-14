//! Register map of the LTR-553ALS-WA proximity and ambient light sensor, and
//! the configuration the capability uses.
//!
//! Values are from the datasheet; each constant says what its bits do. The
//! decoding of the data registers lives in `hack_and_hike_core::light`.

use crate::board::registers::AsyncRegisters;

/// 7-bit I2C address.
pub(super) const ADDRESS: u8 = 0x23;

/// `PART_ID`: part number in the upper nibble, revision in the lower.
pub(super) const PART_ID: u8 = 0x86;
/// The part number of the LTR-553.
pub(super) const PART_NUMBER: u8 = 0x9;

/// `ALS_CONTR`: ambient light sensor gain and mode.
const ALS_CONTR: u8 = 0x80;
/// `ALS_CONTR` bit 0: 1 = active, 0 = standby (the reset state).
const ALS_ACTIVE: u8 = 0x01;
/// `ALS_CONTR` bits 4:2: the gain code. The codes in order of increasing
/// gain: 1x, 2x, 4x, 8x, 48x, 96x (4 and 5 are reserved). 1x spans 1 to
/// 64k lux for daylight; 96x resolves a dark room. The sensor sits behind
/// the tinted front glass, so it needs the high gains indoors.
pub(super) const ALS_GAIN_CODES: [u8; 6] = [0, 1, 2, 3, 6, 7];
/// The gain factor each entry of `ALS_GAIN_CODES` stands for; the sensor
/// reports the factor of every measurement in its status register.
pub(super) const ALS_GAIN_FACTORS: [u8; 6] = [1, 2, 4, 8, 48, 96];
/// Where `ALS_GAIN_CODES` sits in `ALS_CONTR`.
const ALS_GAIN_SHIFT: u8 = 2;

/// `PS_CONTR`: proximity sensor mode.
const PS_CONTR: u8 = 0x81;
/// `PS_CONTR` bits 1:0: `11` = active.
const PS_ACTIVE: u8 = 0x03;

/// `PS_LED`: the infrared LED's pulse frequency (bits 7:5), duty cycle
/// (bits 4:3) and current (bits 2:0).
const PS_LED: u8 = 0x82;
/// 60 kHz (`011`), 100 % duty (`11`), 100 mA (`111`): the reset default.
const PS_LED_DEFAULT: u8 = 0x7F;

/// `PS_N_PULSES`: LED pulses per proximity measurement, 1 to 15. More
/// pulses give a stronger, steadier reflection at the price of LED power;
/// with one pulse (the reset default) a hand at 5 cm read only a few
/// hundred of the 2047 counts.
const PS_N_PULSES: u8 = 0x83;
/// Eight pulses per measurement.
const PS_PULSES: u8 = 8;

/// The raw proximity count with a hand at the edge of the range, about
/// 20 cm from the front: where [`closeness`](hack_and_hike_core::light::closeness_percent)
/// starts. Measured on the CoreS3 with `PS_PULSES` pulses; if the range
/// seems off, read the raw count at 20 cm in the `light_meter` app and put
/// it here.
pub(super) const PROXIMITY_FAR_COUNT: u16 = 50;
/// The raw count with something at the glass: the sensor's maximum.
pub(super) const PROXIMITY_NEAR_COUNT: u16 = hack_and_hike_core::light::PROXIMITY_MAX;

/// `PS_MEAS_RATE`: time between two proximity measurements (bits 3:0).
const PS_MEAS_RATE: u8 = 0x84;
/// 100 ms (`0010`).
const PS_RATE_100MS: u8 = 0x02;

/// `ALS_MEAS_RATE`: integration time (bits 5:3) and time between two
/// ambient light measurements (bits 2:0).
const ALS_MEAS_RATE: u8 = 0x85;
/// Integration 100 ms (`000`), one measurement every 100 ms (`001`).
const ALS_INT_100MS_RATE_100MS: u8 = 0b000_001;
/// The integration time `ALS_INT_100MS_RATE_100MS` stands for, in
/// milliseconds, for the lux formula.
pub(super) const ALS_INTEGRATION_MS: u16 = 100;

/// `ALS_DATA_CH1_0`: first of the seven data registers; see
/// `hack_and_hike_core::light::decode` for their layout.
pub(super) const DATA_START: u8 = 0x88;

/// Put both sensors into active mode, measuring every 100 ms, with the light
/// sensor at the gain `ALS_GAIN_CODES[gain_index]`. The first valid light
/// reading comes one integration time later.
pub(super) async fn configure<I2C: embedded_hal_async::i2c::I2c>(
    registers: &mut AsyncRegisters<'_, I2C>,
    gain_index: usize,
) -> Result<(), I2C::Error> {
    registers.write(PS_LED, PS_LED_DEFAULT).await?;
    registers.write(PS_N_PULSES, PS_PULSES).await?;
    registers.write(PS_MEAS_RATE, PS_RATE_100MS).await?;
    registers
        .write(ALS_MEAS_RATE, ALS_INT_100MS_RATE_100MS)
        .await?;
    registers.write(PS_CONTR, PS_ACTIVE).await?;
    set_als_gain(registers, gain_index).await
}

/// Switch the light sensor to the gain `ALS_GAIN_CODES[gain_index]`, keeping
/// it active. The next measurement is flagged invalid while it settles.
pub(super) async fn set_als_gain<I2C: embedded_hal_async::i2c::I2c>(
    registers: &mut AsyncRegisters<'_, I2C>,
    gain_index: usize,
) -> Result<(), I2C::Error> {
    let code = ALS_GAIN_CODES[gain_index] << ALS_GAIN_SHIFT;
    registers.write(ALS_CONTR, code | ALS_ACTIVE).await
}
