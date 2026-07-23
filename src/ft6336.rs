//! Driver for the FocalTech FT6336 capacitive touch controller (I²C).
//!
//! Register layout (same as the FT6x36 family): `0x02` holds the active
//! touch count (low nibble, up to 2 points), point data starts at `0x03`
//! with 6 bytes per point — XH (event flags in the top nibble, x[11:8] in
//! the low), XL, YH, YL, weight, area. No INT or RST GPIO is wired on this
//! board, so the driver is poll-only; reset rides on the panel reset
//! (TCA9554 EXIO1).

use embedded_hal::i2c::I2c;

pub const ADDR: u8 = 0x38;

const REG_TOUCH_COUNT: u8 = 0x02;
const REG_P1: u8 = 0x03;
const REG_CHIP_ID: u8 = 0xA3;
const REG_FW_VERSION: u8 = 0xA6;
const REG_VENDOR_ID: u8 = 0xA8;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Point {
  pub x: u16,
  pub y: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Touches {
  pub count: u8,
  pub points: [Point; 2],
}

pub struct Ft6336<I2C> {
  i2c: I2C,
}

impl<I2C: I2c> Ft6336<I2C> {
  pub fn new(i2c: I2C) -> Self {
    Self { i2c }
  }

  /// Reads (chip id, firmware version, vendor id) — for logging.
  pub fn info(&mut self) -> Result<(u8, u8, u8), I2C::Error> {
    let mut buf = [0u8; 1];
    self.i2c.write_read(ADDR, &[REG_CHIP_ID], &mut buf)?;
    let chip = buf[0];
    self.i2c.write_read(ADDR, &[REG_FW_VERSION], &mut buf)?;
    let fw = buf[0];
    self.i2c.write_read(ADDR, &[REG_VENDOR_ID], &mut buf)?;
    Ok((chip, fw, buf[0]))
  }

  /// Polls the current touch state (0–2 raw panel-coordinate points).
  pub fn read(&mut self) -> Result<Touches, I2C::Error> {
    let mut count = [0u8; 1];
    self.i2c.write_read(ADDR, &[REG_TOUCH_COUNT], &mut count)?;
    let count = (count[0] & 0x0f).min(2);

    let mut touches = Touches {
      count,
      ..Default::default()
    };
    if count == 0 {
      return Ok(touches);
    }

    let mut data = [0u8; 12];
    let data = &mut data[..6 * count as usize];
    self.i2c.write_read(ADDR, &[REG_P1], data)?;
    for (i, p) in data.chunks_exact(6).enumerate() {
      touches.points[i] = Point {
        x: ((p[0] as u16 & 0x0f) << 8) | p[1] as u16,
        y: ((p[2] as u16 & 0x0f) << 8) | p[3] as u16,
      };
    }
    Ok(touches)
  }

  /// Releases the I²C bus.
  pub fn release(self) -> I2C {
    self.i2c
  }
}
