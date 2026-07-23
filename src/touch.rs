//! FT6336 polling task feeding LVGL's pointer state.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::{Duration, Timer};
use esp_hal::Blocking;
use esp_hal::i2c::master::I2c;
use oxivgl::indev::PointerState;

use crate::ft6336::Ft6336;

/// Consumed by the LVGL pointer indev (registered in `DemoView::create`).
pub static TOUCH_STATE: PointerState = PointerState::new();

/// Latest raw sample for the UI's live-coordinates label, packed as
/// `pressed << 31 | x << 12 | y` (12 bits each side fit 320×480).
pub static TOUCH_XY: AtomicU32 = AtomicU32::new(0);

const PRESSED: u32 = 1 << 31;

pub fn pack(x: u16, y: u16) -> u32 {
  PRESSED | ((x as u32) << 12) | y as u32
}

pub fn unpack(v: u32) -> Option<(u16, u16)> {
  (v & PRESSED != 0).then_some((((v >> 12) & 0xfff) as u16, (v & 0xfff) as u16))
}

/// Polls the FT6336 every 15 ms (~66 Hz), feeding [`TOUCH_STATE`]. Owns the
/// I²C bus exclusively (the TCA9554 releases it after the panel-reset
/// bring-up; revisit sharing when the PMU/RTC/IMU are brought up).
#[embassy_executor::task]
pub async fn touch_task(i2c: I2c<'static, Blocking>) {
  let mut tp = Ft6336::new(i2c);
  match tp.info() {
    Ok((chip, fw, vendor)) => {
      log::info!("FT6336 chip=0x{chip:02x} fw=0x{fw:02x} vendor=0x{vendor:02x}")
    }
    Err(e) => log::error!("FT6336 probe failed: {e:?}"),
  }
  let mut was_down = false;
  loop {
    Timer::after(Duration::from_millis(15)).await;
    match tp.read() {
      Ok(t) if t.count > 0 => {
        let p = t.points[0];
        TOUCH_STATE.touch(p.x, p.y);
        TOUCH_XY.store(pack(p.x, p.y), Ordering::Relaxed);
        was_down = true;
      }
      Ok(_) => {
        if was_down {
          TOUCH_STATE.release();
          TOUCH_XY.fetch_and(!PRESSED, Ordering::Relaxed);
          was_down = false;
        }
      }
      Err(e) => log::warn!("FT6336 poll error: {e:?}"),
    }
  }
}
