#![no_std]
#![no_main]
#![deny(
  clippy::mem_forget,
  reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{DriveMode, Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::interrupt::Priority;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::ledc::channel::ChannelIFace;
use esp_hal::ledc::timer::TimerIFace;
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed, channel, timer};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_rtos::embassy::InterruptExecutor;
use esp32_s3_touch_lcd_3_5::display::{HEIGHT, LVGL_BUF_BYTES, St7796, WIDTH, flush_task};
use esp32_s3_touch_lcd_3_5::sensors::sensor_hub_task;
use esp32_s3_touch_lcd_3_5::tca9554::{EXIO_LCD_RST, Tca9554};
use esp32_s3_touch_lcd_3_5::ui::DemoView;
use esp32_s3_touch_lcd_3_5::wifi;
use oxivgl::display::LvglBuffers;
use oxivgl::view::run_app;
use static_cell::StaticCell;

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new();
// The LEDC channel stores a reference to its timer (and the timer to the
// Ledc block), so both live in StaticCells to make the channel 'static.
static LEDC: StaticCell<Ledc<'static>> = StaticCell::new();
static LEDC_TIMER: StaticCell<timer::Timer<'static, LowSpeed>> = StaticCell::new();

#[embassy_executor::task]
async fn heartbeat() {
  let mut seconds: u32 = 0;
  loop {
    Timer::after(Duration::from_secs(1)).await;
    seconds += 1;
    log::debug!("alive: {}s", seconds);
  }
}

#[allow(
  clippy::large_stack_frames,
  reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
  esp_println::logger::init_logger_from_env();
  let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
  let peripherals = esp_hal::init(config);

  // Internal-RAM global heap (the reclaimed dram2 bootloader region — zero
  // .bss cost — plus a second .bss region: the Wi-Fi driver is the big
  // customer). oxivgl serves LVGL's render scratch from the Rust global
  // allocator; LVGL widget memory comes from its own 48 KiB static pool
  // (LV_MEM_SIZE in lv-conf/lv_conf.h). No PSRAM anywhere: the SPI panel
  // needs no framebuffer — LVGL stripes flush straight out over SPI.
  esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
  esp_alloc::heap_allocator!(size: 64 * 1024);

  let timg0 = TimerGroup::new(peripherals.TIMG0);
  let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
  esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

  spawner.spawn(heartbeat().expect("heartbeat task pool exhausted"));

  // I²C0 bus: the TCA9554 uses it for the panel-reset pulse only, then
  // releases it to the sensor hub task, which owns it exclusively and
  // multiplexes touch/IMU/PMU/RTC polling.
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
  let i2c = expander.release();

  // Backlight: LEDC PWM on GPIO6, 5 kHz / 10-bit (demo parity). The UI's
  // brightness slider drives it through sensors::BACKLIGHT_PCT.
  let ledc = LEDC.init(Ledc::new(peripherals.LEDC));
  ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
  let ledc_timer = LEDC_TIMER.init(ledc.timer::<LowSpeed>(timer::Number::Timer0));
  ledc_timer
    .configure(timer::config::Config {
      duty: timer::config::Duty::Duty10Bit,
      clock_source: timer::LSClockSource::APBClk,
      frequency: Rate::from_khz(5),
    })
    .expect("LEDC timer config failed");
  let mut backlight = ledc.channel::<LowSpeed>(channel::Number::Channel0, peripherals.GPIO6);
  backlight
    .configure(channel::config::Config {
      timer: &*ledc_timer,
      duty_pct: 100,
      drive_mode: DriveMode::PushPull,
    })
    .expect("LEDC channel config failed");

  // ST7796 on SPI2 @ 80 MHz (blocking; stripes are pushed by flush_task).
  let spi = Spi::new(
    peripherals.SPI2,
    SpiConfig::default().with_frequency(Rate::from_mhz(80)),
  )
  .expect("SPI init failed")
  .with_sck(peripherals.GPIO5)
  .with_mosi(peripherals.GPIO1);
  let mut panel = St7796::new(
    spi,
    Output::new(peripherals.GPIO3, Level::High, OutputConfig::default()),
  );
  panel.init(&Delay::new()).expect("panel init failed");
  log::info!("panel initialized");

  // The flush task runs on an interrupt executor: oxivgl's wait callback
  // spins inside the LVGL render task until the flush task acks, so they
  // can never share an executor (and lv_timer_handler blocks the thread
  // executor for tens of ms).
  let int_executor = INT_EXECUTOR.init(InterruptExecutor::new(sw_interrupt.software_interrupt1));
  let hi_spawner = int_executor.start(Priority::min());
  hi_spawner.spawn(flush_task(panel).expect("flush task pool exhausted"));

  // The hub owns the I²C bus (touch + IMU + PMU + RTC) and the backlight.
  spawner.spawn(sensor_hub_task(i2c, backlight).expect("sensor hub task pool exhausted"));

  // Wi-Fi: STA + DHCP when WIFI_SSID/WIFI_PASSWORD were set at build time,
  // scan-only otherwise. Must run after esp_rtos::start and the heap.
  wifi::start(&spawner, peripherals.WIFI);

  // LVGL double render buffers (.bss, internal RAM).
  static mut LVGL_BUFS: LvglBuffers<LVGL_BUF_BYTES> = LvglBuffers::new();
  // SAFETY: accessed exactly once, before the LVGL loop takes ownership.
  let bufs = unsafe { &mut *core::ptr::addr_of_mut!(LVGL_BUFS) };

  run_app::<DemoView, LVGL_BUF_BYTES>(WIDTH as i32, HEIGHT as i32, bufs, DemoView::default()).await
}
