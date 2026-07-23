//! Diagnostic: scan the shared I2C bus and blink the backlight (GPIO6).
//!
//! Flash with `cargo run --release --bin i2c-scan`. Expected ACKs on this
//! board: 0x18 (ES8311), 0x20 (TCA9554), 0x34 (AXP2101), 0x38 (FT6336),
//! 0x51 (PCF85063), 0x6B (QMI8658). The panel backlight should visibly
//! blink at 0.5 Hz (panel content is undefined until the ST7796 is
//! initialized — only the glow matters).

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

const KNOWN: &[(u8, &str)] = &[
  (0x18, "ES8311 audio codec"),
  (0x20, "TCA9554 IO expander"),
  (0x34, "AXP2101 PMU"),
  (0x38, "FT6336 touch"),
  (0x51, "PCF85063 RTC"),
  (0x6B, "QMI8658 IMU"),
];

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
  let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
  let peripherals = esp_hal::init(config);

  let timg0 = TimerGroup::new(peripherals.TIMG0);
  let sw_interrupt =
    esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
  esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

  println!("i2c scan + backlight blink test");

  let mut i2c = I2c::new(
    peripherals.I2C0,
    I2cConfig::default().with_frequency(Rate::from_khz(400)),
  )
  .expect("I2C init failed")
  .with_sda(peripherals.GPIO8)
  .with_scl(peripherals.GPIO7);

  print_scan(&mut i2c);

  let mut backlight = Output::new(peripherals.GPIO6, Level::High, OutputConfig::default());
  let mut on = true;
  loop {
    println!("backlight GPIO6 -> {}", if on { "ON" } else { "OFF" });
    Timer::after(Duration::from_millis(2000)).await;
    on = !on;
    backlight.set_level(if on { Level::High } else { Level::Low });
  }
}

fn print_scan(i2c: &mut I2c<'_, esp_hal::Blocking>) {
  println!("scanning I2C bus (SDA=GPIO8, SCL=GPIO7)...");
  let mut found = 0u32;
  for addr in 0x08..0x78u8 {
    let mut buf = [0u8; 1];
    if i2c.read(addr, &mut buf).is_ok() {
      let name = KNOWN
        .iter()
        .find(|(a, _)| *a == addr)
        .map(|(_, n)| *n)
        .unwrap_or("unknown");
      println!("  ACK at 0x{addr:02x} (read 0x{:02x}) - {name}", buf[0]);
      found += 1;
    }
  }
  println!("scan done, {found} device(s) found (expected 6)");
  for (addr, name) in KNOWN {
    let mut buf = [0u8; 1];
    if i2c.read(*addr, &mut buf).is_err() {
      println!("  MISSING: 0x{addr:02x} {name}");
    }
  }
}
