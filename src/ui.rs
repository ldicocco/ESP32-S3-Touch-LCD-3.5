//! Demo view proving render + touch: a tap-counter button, a slider, and a
//! live touch-coordinate readout. LVGL's built-in FPS/CPU overlay (top-left,
//! enabled via lv_conf.h) doubles as the render-loop health indicator.
//!
//! Note: no color-format override here — oxivgl's default
//! `RGB565_SWAPPED` is exactly the ST7796's SPI wire order (see
//! `display.rs`).

use core::sync::atomic::{AtomicU32, Ordering};

use alloc::format;
use oxivgl::enums::EventCode;
use oxivgl::event::Event;
use oxivgl::fonts;
use oxivgl::indev::PointerIndev;
use oxivgl::style::{Selector, Style};
use oxivgl::view::{NavAction, View};
use oxivgl::widgets::{Align, Button, Label, Obj, Slider, WidgetError};

use crate::touch::{TOUCH_STATE, TOUCH_XY, unpack};
use crate::wifi::{STATE_CONNECTED, STATE_CONNECTING, WIFI_IP, WIFI_STATE};

static TAPS: AtomicU32 = AtomicU32::new(0);

fn on_button_clicked(_event: &Event) {
  TAPS.fetch_add(1, Ordering::Relaxed);
}

#[derive(Default)]
pub struct DemoView {
  /// LVGL pointer input device; registered on first `create` and kept for
  /// the life of the UI (dropping it would unregister the device).
  pointer: Option<PointerIndev>,
  taps_label: Option<Label<'static>>,
  slider: Option<Slider<'static>>,
  slider_label: Option<Label<'static>>,
  coords_label: Option<Label<'static>>,
  wifi_label: Option<Label<'static>>,
  _statics: Option<(Label<'static>, Button<'static>, Label<'static>)>,
  last_taps: u32,
  last_slider: i32,
  last_xy: u32,
  last_wifi: (u8, u32),
}

impl View for DemoView {
  fn create(&mut self, container: &Obj<'static>) -> Result<(), WidgetError> {
    if self.pointer.is_none() {
      self.pointer = Some(PointerIndev::new(&TOUCH_STATE)?);
    }

    let screen_style = Style::new(|s| {
      s.bg_color_hex(0x101828).bg_opa(255).radius(0);
    });
    container.add_style(&screen_style, Selector::DEFAULT);
    container.remove_scrollable();

    let title_style = Style::new(|s| {
      s.text_color_hex(0xffffff).text_font(fonts::MONTSERRAT_20);
    });
    let title = Label::new(container)?;
    title
      .text("ESP32-S3-Touch-LCD-3.5")
      .align(Align::TopMid, 0, 40)
      .add_style(&title_style, Selector::DEFAULT);

    let body_style = Style::new(|s| {
      s.text_color_hex(0xe0e6f0).text_font(fonts::MONTSERRAT_16);
    });

    let button = Button::new(container)?;
    button.size(200, 72).align(Align::TopMid, 0, 110);
    button.on(EventCode::CLICKED, on_button_clicked);
    let button_label = Label::new(&button)?;
    button_label
      .text("Tap me")
      .add_style(&body_style, Selector::DEFAULT)
      .center();

    let taps = Label::new(container)?;
    taps
      .text("Taps: 0")
      .add_style(&body_style, Selector::DEFAULT)
      .align_to(&button, Align::OutBottomMid, 0, 16);

    let slider = Slider::new(container)?;
    slider.set_range(0, 100).set_value(30);
    slider.size(240, 20).align(Align::TopMid, 0, 290);
    let slider_label = Label::new(container)?;
    slider_label
      .text("Slider: 30")
      .add_style(&body_style, Selector::DEFAULT)
      .align_to(&slider, Align::OutBottomMid, 0, 16);

    let wifi = Label::new(container)?;
    wifi
      .text(wifi_text(WIFI_STATE.load(Ordering::Relaxed), 0).as_str())
      .add_style(&body_style, Selector::DEFAULT)
      .align(Align::BottomMid, 0, -72);

    let coords = Label::new(container)?;
    coords
      .text("Touch: -")
      .add_style(&body_style, Selector::DEFAULT)
      .align(Align::BottomMid, 0, -32);

    self.wifi_label = Some(wifi);
    self.taps_label = Some(taps);
    self.slider = Some(slider);
    self.slider_label = Some(slider_label);
    self.coords_label = Some(coords);
    self._statics = Some((title, button, button_label));

    // SAFETY: on the LVGL task; lv_mem_monitor fills the out-struct.
    let lv = unsafe {
      let mut mon = core::mem::MaybeUninit::<oxivgl_sys::lv_mem_monitor_t>::uninit();
      oxivgl_sys::lv_mem_monitor(mon.as_mut_ptr());
      mon.assume_init()
    };
    log::info!(
      "UI created: rust heap used {} B, free {} B; lvgl heap used {} B, free {} B",
      esp_alloc::HEAP.used(),
      esp_alloc::HEAP.free(),
      lv.total_size - lv.free_size,
      lv.free_size
    );
    Ok(())
  }

  fn update(&mut self) -> Result<NavAction, WidgetError> {
    let taps = TAPS.load(Ordering::Relaxed);
    if taps != self.last_taps {
      self.last_taps = taps;
      if let Some(l) = &self.taps_label {
        l.text(&format!("Taps: {taps}"));
      }
    }

    if let (Some(s), Some(l)) = (&self.slider, &self.slider_label) {
      let v = s.get_value();
      if v != self.last_slider {
        self.last_slider = v;
        l.text(&format!("Slider: {v}"));
      }
    }

    let xy = TOUCH_XY.load(Ordering::Relaxed);
    if xy != self.last_xy {
      self.last_xy = xy;
      if let Some(l) = &self.coords_label {
        match unpack(xy) {
          Some((x, y)) => l.text(&format!("Touch: {x}, {y}")),
          None => l.text("Touch: released"),
        };
      }
    }

    let wifi = (
      WIFI_STATE.load(Ordering::Relaxed),
      WIFI_IP.load(Ordering::Relaxed),
    );
    if wifi != self.last_wifi {
      self.last_wifi = wifi;
      if let Some(l) = &self.wifi_label {
        l.text(wifi_text(wifi.0, wifi.1).as_str());
      }
    }
    Ok(NavAction::None)
  }
}

fn wifi_text(state: u8, ip: u32) -> alloc::string::String {
  if ip != 0 {
    let [a, b, c, d] = ip.to_be_bytes();
    return format!("WiFi: {a}.{b}.{c}.{d}");
  }
  match state {
    STATE_CONNECTING => "WiFi: connecting...".into(),
    STATE_CONNECTED => "WiFi: connected (DHCP...)".into(),
    _ => "WiFi: unconfigured".into(),
  }
}
