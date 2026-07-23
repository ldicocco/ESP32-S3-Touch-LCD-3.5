//! Driver for the NXP PCF85063 real-time clock (I²C).
//!
//! Time/date live in BCD registers 0x04–0x0A (seconds → years, 2000-based
//! years 0–99). Bit 7 of the seconds register is the oscillator-stop (OS)
//! flag: set after power loss, meaning the time is not trustworthy.
//! Writing the time (a 7-byte burst starting at seconds) clears OS, so a
//! set operation atomically validates the clock. 24-hour mode (power-on
//! default) is assumed throughout.

use embedded_hal::i2c::I2c;

pub const ADDR: u8 = 0x51;

const REG_CONTROL1: u8 = 0x00;
const REG_SECONDS: u8 = 0x04;

const SECONDS_OS: u8 = 1 << 7;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct DateTime {
  /// Full year, 2000–2099.
  pub year: u16,
  pub month: u8,
  pub day: u8,
  pub hour: u8,
  pub minute: u8,
  pub second: u8,
}

fn bcd_to_bin(b: u8) -> u8 {
  (b >> 4) * 10 + (b & 0x0F)
}

fn bin_to_bcd(b: u8) -> u8 {
  (b / 10) << 4 | (b % 10)
}

pub struct Pcf85063<I2C> {
  i2c: I2C,
}

impl<I2C: I2c> Pcf85063<I2C> {
  pub fn new(i2c: I2C) -> Self {
    Self { i2c }
  }

  /// Raw Control_1 register — for diagnostics (STOP bit, 12/24 mode).
  pub fn control1(&mut self) -> Result<u8, I2C::Error> {
    let mut buf = [0u8; 1];
    self.i2c.write_read(ADDR, &[REG_CONTROL1], &mut buf)?;
    Ok(buf[0])
  }

  /// Reads the time. `valid == false` means the oscillator stopped since
  /// the last set (power loss) and the value can't be trusted.
  pub fn read(&mut self) -> Result<(DateTime, bool), I2C::Error> {
    let mut buf = [0u8; 7];
    self.i2c.write_read(ADDR, &[REG_SECONDS], &mut buf)?;
    let dt = DateTime {
      second: bcd_to_bin(buf[0] & 0x7F),
      minute: bcd_to_bin(buf[1] & 0x7F),
      hour: bcd_to_bin(buf[2] & 0x3F),
      day: bcd_to_bin(buf[3] & 0x3F),
      // buf[4] is the weekday — unused.
      month: bcd_to_bin(buf[5] & 0x1F),
      year: 2000 + bcd_to_bin(buf[6]) as u16,
    };
    Ok((dt, buf[0] & SECONDS_OS == 0))
  }

  /// Sets the time (clears the OS flag as a side effect). Weekday is
  /// written as 0 — nothing on this board consumes it.
  pub fn set(&mut self, dt: &DateTime) -> Result<(), I2C::Error> {
    self.i2c.write(
      ADDR,
      &[
        REG_SECONDS,
        bin_to_bcd(dt.second),
        bin_to_bcd(dt.minute),
        bin_to_bcd(dt.hour),
        bin_to_bcd(dt.day),
        0,
        bin_to_bcd(dt.month),
        bin_to_bcd((dt.year % 100) as u8),
      ],
    )
  }

  /// Releases the I²C bus.
  pub fn release(self) -> I2C {
    self.i2c
  }
}
