//! Sensor hub: one task owns the shared I²C bus and multiplexes all device
//! polling on a 15 ms ticker — touch every tick (unchanged cadence), IMU
//! every 7th (~10 Hz), PMU and RTC staggered at ~1 Hz. Everything the UI
//! needs is published as plain atomics (wifi.rs pattern, polled in
//! `DemoView::update`); the UI talks back through [`BACKLIGHT_PCT`], and
//! the SNTP client hands over timestamps through [`RTC_SET_EPOCH`].
//!
//! Drivers are constructed transiently per poll via the `&mut bus` blanket
//! impl of `embedded_hal::i2c::I2c` — zero-cost, and the hub keeps the only
//! persistent state (availability flags, touch edge state). A device that
//! NACKs at init or fails 5 polls in a row is marked absent and skipped;
//! nothing here panics.
//!
//! No AtomicU64 anywhere — Xtensa has no 64-bit atomics. Per-axis tearing
//! at the 10 Hz IMU publish rate is invisible.

use core::sync::atomic::{AtomicI32, AtomicU8, AtomicU16, AtomicU32, Ordering};

use embassy_time::{Duration, Ticker, Timer};
use esp_hal::Blocking;
use esp_hal::i2c::master::I2c;
use esp_hal::ledc::LowSpeed;
use esp_hal::ledc::channel::{Channel, ChannelIFace};

use crate::axp2101::Axp2101;
use crate::ft6336::Ft6336;
use crate::pcf85063::{DateTime, Pcf85063};
use crate::qmi8658::{self, Qmi8658, accel_mg, gyro_mdps};
use crate::touch::{TOUCH_STATE, TOUCH_XY, pack, release_xy};

/// IMU availability: 0 = absent/failed, 1 = ok.
pub static IMU_STATE: AtomicU8 = AtomicU8::new(0);
/// Wrapping sample counter — the UI diffs this (not the values) to advance
/// its chart, so identical consecutive samples still register.
pub static IMU_SEQ: AtomicU32 = AtomicU32::new(0);
/// Latest acceleration, milli-g, x/y/z.
pub static IMU_ACCEL_MG: [AtomicI32; 3] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
/// Latest angular rate, milli-degrees/s, x/y/z.
pub static IMU_GYRO_MDPS: [AtomicI32; 3] =
  [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];

/// Battery voltage in mV (0 = unknown / no battery).
pub static BAT_MV: AtomicU16 = AtomicU16::new(0);
/// Fuel-gauge percent; [`BAT_PERCENT_UNKNOWN`] = no battery attached.
pub static BAT_PERCENT: AtomicU8 = AtomicU8::new(BAT_PERCENT_UNKNOWN);
pub const BAT_PERCENT_UNKNOWN: u8 = 0xFF;
/// PMU status bits ([`PMU_OK`] etc.).
pub static PMU_FLAGS: AtomicU8 = AtomicU8::new(0);
pub const PMU_OK: u8 = 1 << 0;
pub const PMU_VBUS: u8 = 1 << 1;
pub const PMU_BATTERY: u8 = 1 << 2;
pub const PMU_CHARGING: u8 = 1 << 3;
pub const PMU_CHARGE_DONE: u8 = 1 << 4;

/// RTC time of day, packed `valid << 31 | h << 16 | m << 8 | s`.
pub static RTC_HMS: AtomicU32 = AtomicU32::new(0);
/// RTC date, packed `year << 16 | month << 8 | day` (0 = not read yet).
pub static RTC_DATE: AtomicU32 = AtomicU32::new(0);
/// Set-time command, SNTP → hub: seconds since 2000-01-01 00:00:00 *local*
/// time (0 = no command pending). The hub consumes it with `swap(0)` and
/// writes it to the RTC.
pub static RTC_SET_EPOCH: AtomicU32 = AtomicU32::new(0);

/// Backlight brightness percent: the UI writes it, the hub applies it to
/// the LEDC channel (clamped to ≥ 5 % so the panel can't go irrecoverably
/// dark).
pub static BACKLIGHT_PCT: AtomicU8 = AtomicU8::new(100);

const HMS_VALID: u32 = 1 << 31;

pub fn pack_hms(hour: u8, minute: u8, second: u8) -> u32 {
  HMS_VALID | (hour as u32) << 16 | (minute as u32) << 8 | second as u32
}

/// `Some((h, m, s))` iff the valid bit is set.
pub fn unpack_hms(v: u32) -> Option<(u8, u8, u8)> {
  (v & HMS_VALID != 0).then_some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn pack_date(year: u16, month: u8, day: u8) -> u32 {
  (year as u32) << 16 | (month as u32) << 8 | day as u32
}

/// `Some((year, month, day))` iff a date was published.
pub fn unpack_date(v: u32) -> Option<(u16, u8, u8)> {
  (v != 0).then_some(((v >> 16) as u16, (v >> 8) as u8, v as u8))
}

/// Consecutive-failure tracker: a device that keeps NACKing gets marked
/// absent so the hub stops hammering the bus and spamming the log.
struct Health {
  name: &'static str,
  ok: bool,
  failures: u8,
}

const MAX_FAILURES: u8 = 5;

impl Health {
  fn new(name: &'static str, ok: bool) -> Self {
    Self {
      name,
      ok,
      failures: 0,
    }
  }

  /// Folds one poll result into the tracker; returns `Some(v)` when usable.
  fn check<T, E: core::fmt::Debug>(&mut self, result: Result<T, E>) -> Option<T> {
    match result {
      Ok(v) => {
        self.failures = 0;
        Some(v)
      }
      Err(e) => {
        self.failures += 1;
        log::warn!("{} poll error ({}/{MAX_FAILURES}): {e:?}", self.name, self.failures);
        if self.failures >= MAX_FAILURES {
          self.ok = false;
          log::error!("{} marked unavailable", self.name);
        }
        None
      }
    }
  }
}

const TICK_MS: u64 = 15;
/// IMU cadence: every 7th tick ≈ 105 ms ≈ 10 Hz.
const IMU_EVERY: u32 = 7;
/// PMU/RTC cadence: every 67th tick ≈ 1 s, staggered by half a period.
const SLOW_EVERY: u32 = 67;
const RTC_PHASE: u32 = 33;

/// Minimum applied backlight duty — a slider at 0 must never make the
/// panel unreadable with no way to find the slider again.
const BACKLIGHT_MIN_PCT: u8 = 5;

/// Owns the I²C bus and the backlight channel; see module docs.
#[embassy_executor::task]
pub async fn sensor_hub_task(mut i2c: I2c<'static, Blocking>, backlight: Channel<'static, LowSpeed>) {
  // --- one-time device bring-up (bus is ours alone from here on) ---

  match Ft6336::new(&mut i2c).info() {
    Ok((chip, fw, vendor)) => {
      log::info!("FT6336 chip=0x{chip:02x} fw=0x{fw:02x} vendor=0x{vendor:02x}")
    }
    Err(e) => log::error!("FT6336 probe failed: {e:?}"),
  }
  // Touch stays polled unconditionally — it worked before this hub existed,
  // and a transient probe error must not cost us the input device.
  let mut touch = Health::new("FT6336", true);

  let imu_ok = init_imu(&mut i2c).await;
  let mut imu = Health::new("QMI8658", imu_ok);
  IMU_STATE.store(imu_ok as u8, Ordering::Relaxed);

  let pmu_ok = match Axp2101::new(&mut i2c).init() {
    Ok(()) => true,
    Err(e) => {
      log::warn!("AXP2101 init failed: {e:?}");
      false
    }
  };
  let mut pmu = Health::new("AXP2101", pmu_ok);

  let rtc_ok = match Pcf85063::new(&mut i2c).control1() {
    Ok(c1) => {
      log::info!("PCF85063 control1=0x{c1:02x}");
      true
    }
    Err(e) => {
      log::warn!("PCF85063 probe failed: {e:?}");
      false
    }
  };
  let mut rtc = Health::new("PCF85063", rtc_ok);

  // --- the poll loop ---

  let mut ticker = Ticker::every(Duration::from_millis(TICK_MS));
  let mut tick: u32 = 0;
  let mut was_down = false;
  let mut applied_backlight = u8::MAX; // force one apply on the first tick

  loop {
    ticker.next().await;
    tick = tick.wrapping_add(1);

    if touch.ok
      && let Some(t) = touch.check(Ft6336::new(&mut i2c).read())
    {
      if t.count > 0 {
        let p = t.points[0];
        TOUCH_STATE.touch(p.x, p.y);
        TOUCH_XY.store(pack(p.x, p.y), Ordering::Relaxed);
        was_down = true;
      } else if was_down {
        TOUCH_STATE.release();
        release_xy();
        was_down = false;
      }
    }

    if imu.ok
      && tick % IMU_EVERY == 0
      && let Some(s) = imu.check(Qmi8658::new(&mut i2c).read())
    {
      for (i, &raw) in s.accel.iter().enumerate() {
        IMU_ACCEL_MG[i].store(accel_mg(raw), Ordering::Relaxed);
      }
      for (i, &raw) in s.gyro.iter().enumerate() {
        IMU_GYRO_MDPS[i].store(gyro_mdps(raw), Ordering::Relaxed);
      }
      IMU_SEQ.fetch_add(1, Ordering::Relaxed);
    }
    if !imu.ok {
      IMU_STATE.store(0, Ordering::Relaxed);
    }

    if pmu.ok && tick % SLOW_EVERY == 0 {
      poll_pmu(&mut i2c, &mut pmu);
    }

    if rtc.ok && tick % SLOW_EVERY == RTC_PHASE {
      poll_rtc(&mut i2c, &mut rtc);
    }

    // UI → hub commands, checked every tick (both are register writes /
    // rare one-shot I²C, negligible).
    let wanted = BACKLIGHT_PCT.load(Ordering::Relaxed);
    if wanted != applied_backlight {
      applied_backlight = wanted;
      if let Err(e) = backlight.set_duty(wanted.clamp(BACKLIGHT_MIN_PCT, 100)) {
        log::warn!("backlight set_duty({wanted}) failed: {e:?}");
      }
    }

    let set = RTC_SET_EPOCH.swap(0, Ordering::Relaxed);
    if set != 0 && rtc.ok {
      set_rtc(&mut i2c, &mut rtc, set);
    }
  }
}

/// QMI8658 bring-up: whoami check, reset, settle, configure.
async fn init_imu(i2c: &mut I2c<'static, Blocking>) -> bool {
  match Qmi8658::new(&mut *i2c).info() {
    Ok((chip, rev)) if chip == qmi8658::CHIP_ID => {
      log::info!("QMI8658 whoami=0x{chip:02x} rev=0x{rev:02x}");
      if let Err(e) = Qmi8658::new(&mut *i2c).reset() {
        log::warn!("QMI8658 reset failed: {e:?}");
        return false;
      }
      Timer::after(Duration::from_millis(15)).await;
      match Qmi8658::new(&mut *i2c).configure() {
        Ok(()) => true,
        Err(e) => {
          log::warn!("QMI8658 configure failed: {e:?}");
          false
        }
      }
    }
    Ok((chip, _)) => {
      log::warn!("QMI8658 unexpected whoami 0x{chip:02x}");
      false
    }
    Err(e) => {
      log::warn!("QMI8658 probe failed: {e:?}");
      false
    }
  }
}

fn poll_pmu(i2c: &mut I2c<'static, Blocking>, health: &mut Health) {
  let mut dev = Axp2101::new(i2c);
  let Some(st) = health.check(dev.status()) else {
    return;
  };
  let mut flags = PMU_OK;
  if st.vbus {
    flags |= PMU_VBUS;
  }
  if st.battery {
    flags |= PMU_BATTERY;
    if st.charging {
      flags |= PMU_CHARGING;
    }
    if st.charge_done {
      flags |= PMU_CHARGE_DONE;
    }
    if let Some(mv) = health.check(dev.vbat_mv()) {
      BAT_MV.store(mv, Ordering::Relaxed);
    }
    if let Some(pct) = health.check(dev.battery_percent()) {
      BAT_PERCENT.store(pct.min(100), Ordering::Relaxed);
    }
  } else {
    // Floating ADC without a battery — publish the "unknown" sentinels.
    BAT_MV.store(0, Ordering::Relaxed);
    BAT_PERCENT.store(BAT_PERCENT_UNKNOWN, Ordering::Relaxed);
  }
  PMU_FLAGS.store(flags, Ordering::Relaxed);
}

fn poll_rtc(i2c: &mut I2c<'static, Blocking>, health: &mut Health) {
  let Some((dt, valid)) = health.check(Pcf85063::new(i2c).read()) else {
    return;
  };
  RTC_DATE.store(pack_date(dt.year, dt.month, dt.day), Ordering::Relaxed);
  let hms = pack_hms(dt.hour, dt.minute, dt.second);
  RTC_HMS.store(if valid { hms } else { hms & !HMS_VALID }, Ordering::Relaxed);
}

/// Applies an SNTP time-set command (writing the time also clears the
/// RTC's oscillator-stop flag, validating the clock) and re-publishes.
fn set_rtc(i2c: &mut I2c<'static, Blocking>, health: &mut Health, epoch: u32) {
  let dt = datetime_from_epoch(epoch);
  if health.check(Pcf85063::new(&mut *i2c).set(&dt)).is_some() {
    log::info!(
      "RTC set to {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
      dt.year,
      dt.month,
      dt.day,
      dt.hour,
      dt.minute,
      dt.second
    );
    poll_rtc(i2c, health);
  }
}

/// Civil-date conversion (Howard Hinnant's `civil_from_days`), shifted to
/// the RTC epoch: `epoch` counts seconds since 2000-01-01 00:00:00.
fn datetime_from_epoch(epoch: u32) -> DateTime {
  let days = i64::from(epoch / 86_400);
  let rem = epoch % 86_400;
  // 2000-01-01 is day 730_425 of the proleptic-Gregorian era base
  // 0000-03-01 used by the algorithm.
  let z = days + 730_425;
  let era = z / 146_097;
  let doe = z - era * 146_097;
  let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = doy - (153 * mp + 2) / 5 + 1;
  let (year, month) = if mp < 10 {
    (era * 400 + yoe, mp + 3)
  } else {
    (era * 400 + yoe + 1, mp - 9)
  };
  DateTime {
    year: year as u16,
    month: month as u8,
    day: day as u8,
    hour: (rem / 3_600) as u8,
    minute: ((rem / 60) % 60) as u8,
    second: (rem % 60) as u8,
  }
}
