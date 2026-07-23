//! Touch state shared between the sensor hub and the UI. The FT6336 is
//! polled by `sensors::sensor_hub_task` (which owns the shared I²C bus);
//! this module keeps the published state and its packing helpers.

use core::sync::atomic::{AtomicU32, Ordering};

use oxivgl::indev::PointerState;

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

/// Clears the pressed bit (keeps the last coordinates for the UI label).
pub fn release_xy() {
  TOUCH_XY.fetch_and(!PRESSED, Ordering::Relaxed);
}
