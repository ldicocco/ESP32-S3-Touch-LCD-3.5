//! ST7796 panel driver (SPI, blocking) for the 3.5-inch 320×480 IPS panel.
//!
//! Wiring on this board: SPI2 @ 80 MHz mode 0 (SCLK GPIO5, MOSI GPIO1, CS
//! hard-wired active), DC on GPIO3, panel reset via TCA9554 EXIO1 (pulse
//! before calling [`St7796::init`]), backlight on GPIO6.
//!
//! The init sequence is Waveshare's, taken verbatim from the vendored
//! `esp_lcd_st7796` component in their demo repo (the panel-specific gamma
//! / power tables differ from Espressif's upstream defaults). Color format
//! is RGB565 with the BGR order bit and inversion on, matching the demo
//! (`LCD_RGB_ELEMENT_ORDER_BGR` + `invert_color(true)`).

use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::gpio::Output;
use esp_hal::spi::Error as SpiError;
use esp_hal::spi::master::Spi;
use oxivgl::flush_pipeline::{DisplayOutput, UiError, flush_frame_buffer};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 480;

/// Rows per LVGL render stripe (per draw buffer): 40 rows = 12 stripes per
/// full refresh, 2 × 25 KiB of .bss. Each stripe is one blocking SPI burst
/// (~2.6 ms at 80 MHz); upgrade path is DMA + async SPI if flush time ever
/// dominates.
pub const LVGL_BUF_LINES: usize = 40;
pub const LVGL_BUF_BYTES: usize = WIDTH * LVGL_BUF_LINES * 2;

// MADCTL: MX (mirror X, matching the demo's rotation-0 config) + BGR.
const MADCTL_PORTRAIT: u8 = 0x40 | 0x08;

pub struct St7796<'d> {
  spi: Spi<'d, Blocking>,
  dc: Output<'d>,
}

impl<'d> St7796<'d> {
  pub fn new(spi: Spi<'d, Blocking>, dc: Output<'d>) -> Self {
    Self { spi, dc }
  }

  fn cmd(&mut self, cmd: u8, params: &[u8]) -> Result<(), SpiError> {
    self.dc.set_low();
    self.spi.write(&[cmd])?;
    self.dc.set_high();
    if !params.is_empty() {
      self.spi.write(params)?;
    }
    Ok(())
  }

  /// Full panel init. The hardware reset (TCA9554 EXIO1 pulse) must have
  /// happened already.
  pub fn init(&mut self, delay: &Delay) -> Result<(), SpiError> {
    self.cmd(0x01, &[])?; // SWRESET
    delay.delay_millis(120);
    self.cmd(0x11, &[])?; // SLPOUT
    delay.delay_millis(120);

    self.cmd(0x36, &[MADCTL_PORTRAIT])?;
    self.cmd(0x3A, &[0x05])?; // COLMOD: 16 bpp

    // Waveshare vendor sequence (command-set unlock, power, gamma, lock).
    self.cmd(0xF0, &[0xC3])?;
    self.cmd(0xF0, &[0x96])?;
    self.cmd(0xB4, &[0x01])?;
    self.cmd(0xB7, &[0xC6])?;
    self.cmd(0xC0, &[0x80, 0x45])?;
    self.cmd(0xC1, &[0x13])?;
    self.cmd(0xC2, &[0xA7])?;
    self.cmd(0xC5, &[0x0A])?;
    self.cmd(0xE8, &[0x40, 0x8A, 0x00, 0x00, 0x29, 0x19, 0xA5, 0x33])?;
    self.cmd(
      0xE0,
      &[
        0xD0, 0x08, 0x0F, 0x06, 0x06, 0x33, 0x30, 0x33, 0x47, 0x17, 0x13, 0x13, 0x2B, 0x31,
      ],
    )?;
    self.cmd(
      0xE1,
      &[
        0xD0, 0x0A, 0x11, 0x0B, 0x09, 0x07, 0x2F, 0x33, 0x47, 0x38, 0x15, 0x16, 0x2C, 0x32,
      ],
    )?;
    self.cmd(0xF0, &[0x3C])?;
    self.cmd(0xF0, &[0x69])?;
    delay.delay_millis(120);

    self.cmd(0x21, &[])?; // INVON
    self.cmd(0x29, &[])?; // DISPON
    delay.delay_millis(20);
    Ok(())
  }

  /// Sets the drawing window (inclusive coordinates) and starts a RAM
  /// write; follow with [`St7796::push_pixels`] for exactly
  /// `(x1-x0+1)*(y1-y0+1)` pixels.
  pub fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) -> Result<(), SpiError> {
    let [x0h, x0l] = x0.to_be_bytes();
    let [x1h, x1l] = x1.to_be_bytes();
    let [y0h, y0l] = y0.to_be_bytes();
    let [y1h, y1l] = y1.to_be_bytes();
    self.cmd(0x2A, &[x0h, x0l, x1h, x1l])?; // CASET
    self.cmd(0x2B, &[y0h, y0l, y1h, y1l])?; // RASET
    self.cmd(0x2C, &[]) // RAMWR
  }

  /// Streams pixel data (RGB565, big-endian byte pairs) into the window
  /// opened by [`St7796::set_window`]. May be called repeatedly.
  pub fn push_pixels(&mut self, bytes: &[u8]) -> Result<(), SpiError> {
    self.spi.write(bytes)
  }
}

/// RGB565 color as the big-endian byte pair the panel expects on the wire.
pub const fn rgb565_bytes(color: u16) -> [u8; 2] {
  color.to_be_bytes()
}

/// oxivgl flush endpoint: each LVGL stripe goes straight out over SPI.
///
/// oxivgl registers the display as `LV_COLOR_FORMAT_RGB565_SWAPPED`, i.e.
/// LVGL renders each pixel as a big-endian byte pair — exactly the ST7796's
/// wire order, so the buffer is written verbatim (no per-pixel swap, no
/// color-format override; contrast with the 5-inch DPI panel, which needs
/// the native-RGB565 override in its `ui.rs`).
impl DisplayOutput for St7796<'static> {
  async fn show_raw_data(
    &mut self,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    data: &[u8],
  ) -> Result<(), UiError> {
    let bytes = w as usize * h as usize * 2;
    if x as usize + w as usize > WIDTH
      || y as usize + h as usize > HEIGHT
      || w == 0
      || h == 0
      || data.len() < bytes
    {
      return Err(UiError::Display);
    }
    self
      .set_window(x, y, x + w - 1, y + h - 1)
      .and_then(|()| self.push_pixels(&data[..bytes]))
      .map_err(|_| UiError::Display)
  }
}

/// Drains LVGL's flush channel into the panel. Runs on the interrupt
/// executor: oxivgl's wait callback spins inside the LVGL render task until
/// the flush task acks, so they can never share an executor.
#[embassy_executor::task]
pub async fn flush_task(panel: St7796<'static>) -> ! {
  flush_frame_buffer(panel).await
}
