//! Diagnostic: bring up the QMI8658 IMU, AXP2101 PMU, and PCF85063 RTC on
//! the shared I²C bus and print their readings once per second.
//!
//! Flash with `cargo run --release --bin sensor-test`. Expected at rest on
//! USB power: gravity ≈ ±1000 mg on exactly one accel axis, gyro near 0,
//! VBUS ≈ 5000 mV, "no battery" flags, and a ticking RTC time (OS flag set
//! until the clock is first written after battery/power loss).
//!
//! Set the RTC once at boot by baking a timestamp into the build (the
//! wifi-credentials pattern):
//!
//! ```sh
//! RTC_SET="2026-07-23 14:00:00" cargo run --release --bin sensor-test
//! ```

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp32_s3_touch_lcd_3_5::axp2101::Axp2101;
use esp32_s3_touch_lcd_3_5::pcf85063::{DateTime, Pcf85063};
use esp32_s3_touch_lcd_3_5::qmi8658::{self, Qmi8658, accel_mg, gyro_mdps};

esp_bootloader_esp_idf::esp_app_desc!();

const RTC_SET: Option<&str> = option_env!("RTC_SET");

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
  let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
  let peripherals = esp_hal::init(config);

  let timg0 = TimerGroup::new(peripherals.TIMG0);
  let sw_interrupt =
    esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
  esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

  println!("sensor test: QMI8658 (IMU) + AXP2101 (PMU) + PCF85063 (RTC)");

  let mut i2c = I2c::new(
    peripherals.I2C0,
    I2cConfig::default().with_frequency(Rate::from_khz(400)),
  )
  .expect("I2C init failed")
  .with_sda(peripherals.GPIO8)
  .with_scl(peripherals.GPIO7);

  // --- QMI8658 ---
  let mut imu_ok = false;
  match Qmi8658::new(&mut i2c).info() {
    Ok((chip, rev)) => {
      println!(
        "QMI8658 whoami=0x{chip:02x} (expect 0x{:02x}) rev=0x{rev:02x}",
        qmi8658::CHIP_ID
      );
      if chip == qmi8658::CHIP_ID {
        Qmi8658::new(&mut i2c).reset().expect("IMU reset failed");
        Timer::after(Duration::from_millis(15)).await;
        Qmi8658::new(&mut i2c)
          .configure()
          .expect("IMU configure failed");
        imu_ok = true;
      }
    }
    Err(e) => println!("QMI8658 probe FAILED: {e:?}"),
  }

  // --- AXP2101 ---
  let mut pmu_ok = false;
  {
    let mut pmu = Axp2101::new(&mut i2c);
    match pmu.status_raw() {
      Ok((s1, s2)) => {
        println!("AXP2101 status1=0x{s1:02x} status2=0x{s2:02x}");
        pmu.init().expect("PMU ADC init failed");
        pmu_ok = true;
      }
      Err(e) => println!("AXP2101 probe FAILED: {e:?}"),
    }
  }

  // --- PCF85063 ---
  let mut rtc_ok = false;
  {
    let mut rtc = Pcf85063::new(&mut i2c);
    match rtc.control1() {
      Ok(c1) => {
        println!("PCF85063 control1=0x{c1:02x}");
        rtc_ok = true;
        if let Some(dt) = RTC_SET.and_then(parse_datetime) {
          rtc.set(&dt).expect("RTC set failed");
          println!(
            "RTC set to {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
          );
        } else if RTC_SET.is_some() {
          println!("RTC_SET present but unparseable (want \"YYYY-MM-DD HH:MM:SS\")");
        }
      }
      Err(e) => println!("PCF85063 probe FAILED: {e:?}"),
    }
  }

  loop {
    Timer::after(Duration::from_secs(1)).await;

    if imu_ok {
      let mut imu = Qmi8658::new(&mut i2c);
      match imu.read() {
        Ok(s) => println!(
          "IMU accel mg [{} {} {}]  gyro mdps [{} {} {}]",
          accel_mg(s.accel[0]),
          accel_mg(s.accel[1]),
          accel_mg(s.accel[2]),
          gyro_mdps(s.gyro[0]),
          gyro_mdps(s.gyro[1]),
          gyro_mdps(s.gyro[2]),
        ),
        Err(e) => println!("IMU read error: {e:?}"),
      }
    }

    if pmu_ok {
      let mut pmu = Axp2101::new(&mut i2c);
      match (pmu.status(), pmu.vbat_mv(), pmu.vbus_mv(), pmu.battery_percent()) {
        (Ok(st), Ok(vbat), Ok(vbus), Ok(pct)) => println!(
          "PMU vbus={vbus} mV vbat={vbat} mV pct={pct}% [vbus_good={} battery={} charging={} done={}]",
          st.vbus, st.battery, st.charging, st.charge_done
        ),
        (st, vbat, vbus, pct) => {
          println!("PMU read error: st={st:?} vbat={vbat:?} vbus={vbus:?} pct={pct:?}")
        }
      }
    }

    if rtc_ok {
      let mut rtc = Pcf85063::new(&mut i2c);
      match rtc.read() {
        Ok((dt, valid)) => println!(
          "RTC {:04}-{:02}-{:02} {:02}:{:02}:{:02} valid={valid}",
          dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
        ),
        Err(e) => println!("RTC read error: {e:?}"),
      }
    }
  }
}

/// Parses "YYYY-MM-DD HH:MM:SS". Returns `None` on any malformation.
fn parse_datetime(s: &str) -> Option<DateTime> {
  let (date, time) = s.trim().split_once(' ')?;
  let mut date = date.split('-');
  let mut time = time.split(':');
  let dt = DateTime {
    year: date.next()?.parse().ok()?,
    month: date.next()?.parse().ok()?,
    day: date.next()?.parse().ok()?,
    hour: time.next()?.parse().ok()?,
    minute: time.next()?.parse().ok()?,
    second: time.next()?.parse().ok()?,
  };
  ((2000..2100).contains(&dt.year)
    && (1..=12).contains(&dt.month)
    && (1..=31).contains(&dt.day)
    && dt.hour < 24
    && dt.minute < 60
    && dt.second < 60)
    .then_some(dt)
}
