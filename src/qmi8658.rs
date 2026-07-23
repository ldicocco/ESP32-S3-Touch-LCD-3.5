//! Driver for the QST QMI8658 6-axis IMU (accelerometer + gyroscope, I²C).
//!
//! Minimal bring-up: reset, whoami check, fixed configuration (±4 g accel,
//! ±512 dps gyro, ~125 Hz ODR on both), and a 12-byte burst read of the six
//! little-endian i16 output registers starting at `AX_L` (0x35). Register
//! values follow the QMI8658A datasheet / QST reference driver; the burst
//! data is little-endian on I²C (CTRL1's BE bit affects SPI only).
//!
//! Reset needs ~15 ms before configuration — the driver splits `reset()`
//! and `configure()` so the caller provides the wait (async or Delay).

use embedded_hal::i2c::I2c;

pub const ADDR: u8 = 0x6B;
/// Expected WHO_AM_I value.
pub const CHIP_ID: u8 = 0x05;

const REG_WHO_AM_I: u8 = 0x00;
const REG_REVISION: u8 = 0x01;
const REG_CTRL1: u8 = 0x02;
const REG_CTRL2: u8 = 0x03;
const REG_CTRL3: u8 = 0x04;
const REG_CTRL7: u8 = 0x08;
const REG_AX_L: u8 = 0x35;
const REG_RESET: u8 = 0x60;

/// CTRL1: register address auto-increment for burst reads.
const CTRL1_ADDR_AI: u8 = 0x40;
/// CTRL2: aFS = ±4 g (001 << 4), aODR = 125 Hz (0110).
const CTRL2_4G_125HZ: u8 = 0x16;
/// CTRL3: gFS = ±512 dps (101 << 4), gODR = 125 Hz (0110).
const CTRL3_512DPS_125HZ: u8 = 0x56;
/// CTRL7: aEN | gEN.
const CTRL7_ACCEL_GYRO_EN: u8 = 0x03;
const RESET_COMMAND: u8 = 0xB0;

/// Full-scale ranges matching the CTRL2/CTRL3 config above.
const ACCEL_FS_MG: i32 = 4000; // ±4 g
const GYRO_FS_MDPS: i32 = 512_000; // ±512 dps

/// One raw sample: accel + gyro, x/y/z.
#[derive(Clone, Copy, Default, Debug)]
pub struct Sample {
  pub accel: [i16; 3],
  pub gyro: [i16; 3],
}

/// Raw accel count → milli-g at the driver's ±4 g setting.
pub fn accel_mg(raw: i16) -> i32 {
  raw as i32 * ACCEL_FS_MG / 32768
}

/// Raw gyro count → milli-degrees-per-second at ±512 dps.
pub fn gyro_mdps(raw: i16) -> i32 {
  raw as i32 * GYRO_FS_MDPS / 32768
}

pub struct Qmi8658<I2C> {
  i2c: I2C,
}

impl<I2C: I2c> Qmi8658<I2C> {
  pub fn new(i2c: I2C) -> Self {
    Self { i2c }
  }

  /// Reads (WHO_AM_I, revision) — whoami must be [`CHIP_ID`].
  pub fn info(&mut self) -> Result<(u8, u8), I2C::Error> {
    Ok((self.read_reg(REG_WHO_AM_I)?, self.read_reg(REG_REVISION)?))
  }

  /// Issues a soft reset. Wait ~15 ms before calling [`configure`].
  ///
  /// [`configure`]: Self::configure
  pub fn reset(&mut self) -> Result<(), I2C::Error> {
    self.i2c.write(ADDR, &[REG_RESET, RESET_COMMAND])
  }

  /// Applies the fixed config: auto-increment, ±4 g / ±512 dps @ 125 Hz,
  /// accel + gyro enabled.
  pub fn configure(&mut self) -> Result<(), I2C::Error> {
    self.i2c.write(ADDR, &[REG_CTRL1, CTRL1_ADDR_AI])?;
    self.i2c.write(ADDR, &[REG_CTRL2, CTRL2_4G_125HZ])?;
    self.i2c.write(ADDR, &[REG_CTRL3, CTRL3_512DPS_125HZ])?;
    self.i2c.write(ADDR, &[REG_CTRL7, CTRL7_ACCEL_GYRO_EN])
  }

  /// Burst-reads the latest accel + gyro sample (12 bytes from AX_L).
  pub fn read(&mut self) -> Result<Sample, I2C::Error> {
    let mut buf = [0u8; 12];
    self.i2c.write_read(ADDR, &[REG_AX_L], &mut buf)?;
    let word = |i: usize| i16::from_le_bytes([buf[i], buf[i + 1]]);
    Ok(Sample {
      accel: [word(0), word(2), word(4)],
      gyro: [word(6), word(8), word(10)],
    })
  }

  /// Releases the I²C bus.
  pub fn release(self) -> I2C {
    self.i2c
  }

  fn read_reg(&mut self, reg: u8) -> Result<u8, I2C::Error> {
    let mut buf = [0u8; 1];
    self.i2c.write_read(ADDR, &[reg], &mut buf)?;
    Ok(buf[0])
  }
}
