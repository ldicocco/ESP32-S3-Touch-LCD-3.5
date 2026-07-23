//! Diagnostic: poll the FT6336 touch controller and print coordinates.
//!
//! Flash with `cargo run --release --bin touch-test`, then touch the
//! panel: every press/move/release is logged with raw panel coordinates
//! (x 0..319, y 0..479). The panel itself is left uninitialized — only
//! the backlight is switched on so it's obvious the board is alive.

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
use esp32_s3_touch_lcd_3_5::ft6336::{Ft6336, Touches};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
  let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
  let peripherals = esp_hal::init(config);

  let timg0 = TimerGroup::new(peripherals.TIMG0);
  let sw_interrupt =
    esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
  esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

  println!("FT6336 touch test - touch the panel");

  let _backlight = Output::new(peripherals.GPIO6, Level::High, OutputConfig::default());

  let i2c = I2c::new(
    peripherals.I2C0,
    I2cConfig::default().with_frequency(Rate::from_khz(400)),
  )
  .expect("I2C init failed")
  .with_sda(peripherals.GPIO8)
  .with_scl(peripherals.GPIO7);

  let mut touch = Ft6336::new(i2c);
  match touch.info() {
    Ok((chip, fw, vendor)) => {
      println!("FT6336 chip=0x{chip:02x} fw=0x{fw:02x} vendor=0x{vendor:02x}")
    }
    Err(e) => println!("FT6336 info read failed: {e:?}"),
  }

  let mut last = Touches::default();
  loop {
    Timer::after(Duration::from_millis(20)).await;
    let touches = match touch.read() {
      Ok(t) => t,
      Err(e) => {
        println!("touch read failed: {e:?}");
        continue;
      }
    };
    if touches == last {
      continue;
    }
    match touches.count {
      0 => println!("release"),
      1 => {
        let p = touches.points[0];
        println!("touch ({}, {})", p.x, p.y);
      }
      _ => {
        let (a, b) = (touches.points[0], touches.points[1]);
        println!("touch ({}, {}) + ({}, {})", a.x, a.y, b.x, b.y);
      }
    }
    last = touches;
  }
}
