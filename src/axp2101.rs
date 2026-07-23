//! Driver for the X-Powers AXP2101 PMU (I²C) — read-mostly minimal subset.
//!
//! The board boots fine on PMU hardware defaults, so `init()` deliberately
//! writes a single register: ADC channel control (0x30) — VBAT/VBUS/VSYS
//! measurement on and, crucially, **TS-pin measurement off** (the battery
//! connector has no NTC; the charger misbehaves if TS measure stays on once
//! a battery is attached). Rails, charge current, and CV target stay on
//! hardware defaults — the regs to visit when battery/camera/audio matter:
//! 0x61 (pre-charge), 0x62 (constant current), 0x64 (CV target).
//!
//! Register/bit layout follows XPowersLib (the reference driver); the
//! status bits are hardware-verified via `--bin sensor-test`.

use embedded_hal::i2c::I2c;

pub const ADDR: u8 = 0x34;

const REG_STATUS1: u8 = 0x00;
const REG_STATUS2: u8 = 0x01;
const REG_ADC_CTRL: u8 = 0x30;
const REG_VBAT_H: u8 = 0x34;
const REG_VBUS_H: u8 = 0x38;
const REG_BAT_PERCENT: u8 = 0xA4;

/// ADC ctrl: VSYS | VBUS | VBAT enabled, TS measure disabled.
const ADC_VBAT_VBUS_VSYS: u8 = 0x0D;

const STATUS1_VBUS_GOOD: u8 = 1 << 5;
const STATUS1_BAT_PRESENT: u8 = 1 << 3;

/// STATUS2 bits 2:0 — charge status field values.
const CHG_STATUS_DONE: u8 = 4;

#[derive(Clone, Copy, Default, Debug)]
pub struct Status {
  /// USB (VBUS) power present and good.
  pub vbus: bool,
  /// A battery is connected.
  pub battery: bool,
  /// Battery is currently being charged.
  pub charging: bool,
  /// Charge cycle completed.
  pub charge_done: bool,
}

pub struct Axp2101<I2C> {
  i2c: I2C,
}

impl<I2C: I2c> Axp2101<I2C> {
  pub fn new(i2c: I2C) -> Self {
    Self { i2c }
  }

  /// The whole init: enable VBAT/VBUS/VSYS ADC channels, disable TS-pin
  /// measurement. Everything else stays on hardware defaults.
  pub fn init(&mut self) -> Result<(), I2C::Error> {
    self.i2c.write(ADDR, &[REG_ADC_CTRL, ADC_VBAT_VBUS_VSYS])
  }

  /// Decoded power/charge status from STATUS1/STATUS2.
  pub fn status(&mut self) -> Result<Status, I2C::Error> {
    let (s1, s2) = self.status_raw()?;
    // STATUS2 bits 6:5: current direction (01 = charging); bits 2:0: charge
    // status (0..3 = charge phases, 4 = done, 5 = not charging).
    Ok(Status {
      vbus: s1 & STATUS1_VBUS_GOOD != 0,
      battery: s1 & STATUS1_BAT_PRESENT != 0,
      charging: (s2 >> 5) & 0x03 == 0x01,
      charge_done: s2 & 0x07 == CHG_STATUS_DONE,
    })
  }

  /// Raw (STATUS1, STATUS2) for diagnostics.
  pub fn status_raw(&mut self) -> Result<(u8, u8), I2C::Error> {
    Ok((self.read_reg(REG_STATUS1)?, self.read_reg(REG_STATUS2)?))
  }

  /// Battery voltage in mV (14-bit ADC, 1 mV/LSB). 0 without a battery.
  pub fn vbat_mv(&mut self) -> Result<u16, I2C::Error> {
    self.read_mv(REG_VBAT_H)
  }

  /// VBUS voltage in mV — ~5000 on USB power, a handy sanity value.
  pub fn vbus_mv(&mut self) -> Result<u16, I2C::Error> {
    self.read_mv(REG_VBUS_H)
  }

  /// Fuel-gauge state of charge, 0–100. Garbage without a battery — gate
  /// on [`Status::battery`].
  pub fn battery_percent(&mut self) -> Result<u8, I2C::Error> {
    self.read_reg(REG_BAT_PERCENT)
  }

  /// Releases the I²C bus.
  pub fn release(self) -> I2C {
    self.i2c
  }

  fn read_mv(&mut self, reg_h: u8) -> Result<u16, I2C::Error> {
    let mut buf = [0u8; 2];
    self.i2c.write_read(ADDR, &[reg_h], &mut buf)?;
    Ok(((buf[0] as u16 & 0x3F) << 8) | buf[1] as u16)
  }

  fn read_reg(&mut self, reg: u8) -> Result<u8, I2C::Error> {
    let mut buf = [0u8; 1];
    self.i2c.write_read(ADDR, &[reg], &mut buf)?;
    Ok(buf[0])
  }
}
