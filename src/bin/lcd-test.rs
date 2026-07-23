//! Diagnostic: bring up the ST7796 panel and draw color bars (no LVGL).
//!
//! Flash with `cargo run --release --bin lcd-test`. Expected result: eight
//! horizontal bands, top to bottom: WHITE, RED, GREEN, BLUE, YELLOW, CYAN,
//! MAGENTA, BLACK. If red and blue are swapped, the MADCTL BGR bit in
//! `src/display.rs` is wrong for this panel.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp32_s3_touch_lcd_3_5::display::{HEIGHT, St7796, WIDTH, rgb565_bytes};
use esp32_s3_touch_lcd_3_5::tca9554::{EXIO_LCD_RST, Tca9554};

esp_bootloader_esp_idf::esp_app_desc!();

const BARS: &[(u16, &str)] = &[
  (0xFFFF, "white"),
  (0xF800, "red"),
  (0x07E0, "green"),
  (0x001F, "blue"),
  (0xFFE0, "yellow"),
  (0x07FF, "cyan"),
  (0xF81F, "magenta"),
  (0x0000, "black"),
];

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
  let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
  let peripherals = esp_hal::init(config);

  let timg0 = TimerGroup::new(peripherals.TIMG0);
  let sw_interrupt =
    esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
  esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

  println!("ST7796 color-bar test");

  // Panel hardware reset via the IO expander (demo timing: 100 ms pulse).
  let i2c = I2c::new(
    peripherals.I2C0,
    I2cConfig::default().with_frequency(Rate::from_khz(400)),
  )
  .expect("I2C init failed")
  .with_sda(peripherals.GPIO8)
  .with_scl(peripherals.GPIO7);
  let mut expander = Tca9554::new(i2c);
  expander
    .set_direction_output(EXIO_LCD_RST)
    .expect("TCA9554 unreachable");
  expander.set_output(EXIO_LCD_RST, false).unwrap();
  Timer::after(Duration::from_millis(100)).await;
  expander.set_output(EXIO_LCD_RST, true).unwrap();
  Timer::after(Duration::from_millis(100)).await;
  println!("panel reset pulsed (TCA9554 EXIO1)");

  let _backlight = Output::new(peripherals.GPIO6, Level::High, OutputConfig::default());

  let spi = Spi::new(
    peripherals.SPI2,
    SpiConfig::default().with_frequency(Rate::from_mhz(80)),
  )
  .expect("SPI init failed")
  .with_sck(peripherals.GPIO5)
  .with_mosi(peripherals.GPIO1);

  let delay = Delay::new();
  let mut panel = St7796::new(
    spi,
    Output::new(peripherals.GPIO3, Level::High, OutputConfig::default()),
  );
  panel.init(&delay).expect("panel init failed");
  println!("panel initialized");

  panel
    .set_window(0, 0, (WIDTH - 1) as u16, (HEIGHT - 1) as u16)
    .expect("set_window failed");
  let rows_per_bar = HEIGHT / BARS.len();
  let mut line = [0u8; WIDTH * 2];
  for (color, name) in BARS {
    let [hi, lo] = rgb565_bytes(*color);
    for px in line.chunks_exact_mut(2) {
      px[0] = hi;
      px[1] = lo;
    }
    for _ in 0..rows_per_bar {
      panel.push_pixels(&line).expect("pixel write failed");
    }
    println!("bar: {name}");
  }
  println!("color bars drawn, check the panel");

  loop {
    Timer::after(Duration::from_secs(5)).await;
    println!("lcd-test idle");
  }
}
