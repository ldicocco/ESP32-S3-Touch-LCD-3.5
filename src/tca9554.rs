//! Driver for the TI TCA9554 I²C IO expander (8-bit, register-based).
//!
//! Conventional register-pointer device at address `0x20` (A2..A0 = 000):
//!
//! | Reg | Name     | Function                              |
//! |-----|----------|---------------------------------------|
//! | 0x00| INPUT    | pin levels (read)                     |
//! | 0x01| OUTPUT   | output latch (power-on 0xFF)          |
//! | 0x02| POLARITY | input polarity inversion (unused here)|
//! | 0x03| CONFIG   | 1 = input (power-on), 0 = output      |
//!
//! Output/config shadows are kept so per-pin updates don't need reads.

use embedded_hal::i2c::I2c;

const ADDR: u8 = 0x20;

const REG_INPUT: u8 = 0x00;
const REG_OUTPUT: u8 = 0x01;
const REG_CONFIG: u8 = 0x03;

// ESP32-S3-Touch-LCD-3.5 EXIO assignments (only EXIO1 is demo-verified;
// check the schematic before assuming the rest).
pub const EXIO_LCD_RST: u8 = 1;

pub struct Tca9554<I2C> {
  i2c: I2C,
  output: u8,
  config: u8,
}

impl<I2C: I2c> Tca9554<I2C> {
  /// Wraps the bus without touching the chip: shadows start at the
  /// power-on defaults (all inputs, output latch all-high).
  pub fn new(i2c: I2C) -> Self {
    Self {
      i2c,
      output: 0xff,
      config: 0xff,
    }
  }

  /// Configures a pin as a push-pull output (its latch level applies).
  pub fn set_direction_output(&mut self, pin: u8) -> Result<(), I2C::Error> {
    debug_assert!(pin < 8);
    self.config &= !(1 << pin);
    self.i2c.write(ADDR, &[REG_CONFIG, self.config])
  }

  /// Sets an output pin's latch level.
  pub fn set_output(&mut self, pin: u8, level: bool) -> Result<(), I2C::Error> {
    debug_assert!(pin < 8);
    if level {
      self.output |= 1 << pin;
    } else {
      self.output &= !(1 << pin);
    }
    self.i2c.write(ADDR, &[REG_OUTPUT, self.output])
  }

  /// Reads the input port register (levels of all 8 pins).
  pub fn read_inputs(&mut self) -> Result<u8, I2C::Error> {
    let mut buf = [0u8; 1];
    self.i2c.write_read(ADDR, &[REG_INPUT], &mut buf)?;
    Ok(buf[0])
  }

  /// Releases the I²C bus (outputs keep their last written state).
  pub fn release(self) -> I2C {
    self.i2c
  }
}
