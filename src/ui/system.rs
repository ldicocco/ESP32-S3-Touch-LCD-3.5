//! System tab: backlight brightness slider (writes
//! `sensors::BACKLIGHT_PCT`, the sensor hub applies the PWM) and a live
//! stats table (network, LVGL pool, Rust heap, uptime).

use core::sync::atomic::Ordering;

use alloc::format;

use oxivgl::layout::{FlexAlign, FlexFlow};
use oxivgl::style::{Selector, lv_pct};
use oxivgl::widgets::{Label, Obj, Slider, Table, WidgetError};

use crate::sensors::BACKLIGHT_PCT;
use crate::ui::{Theme, wifi_text};
use crate::wifi::{WIFI_IP, WIFI_STATE};

const ROW_IP: u32 = 0;
const ROW_WIFI: u32 = 1;
const ROW_LVGL_USED: u32 = 2;
const ROW_LVGL_FREE: u32 = 3;
const ROW_HEAP_USED: u32 = 4;
const ROW_HEAP_FREE: u32 = 5;
const ROW_UPTIME: u32 = 6;
const ROW_CHIP: u32 = 7;

#[derive(Default)]
pub struct SystemTab {
  bl_slider: Option<Slider<'static>>,
  bl_label: Option<Label<'static>>,
  table: Option<Table<'static>>,
  _statics: Option<(Label<'static>, Label<'static>)>,
  last_bl: i32,
}

impl SystemTab {
  pub fn create(&mut self, pane: &Obj<'static>, theme: &Theme) -> Result<(), WidgetError> {
    pane.add_style(&theme.pane, Selector::DEFAULT);
    pane.set_flex_flow(FlexFlow::Column);
    pane.set_flex_align(FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);

    let caption = Label::new(pane)?;
    caption
      .text("Backlight")
      .add_style(&theme.title, Selector::DEFAULT);

    let init_pct = BACKLIGHT_PCT.load(Ordering::Relaxed) as i32;
    let bl_slider = Slider::new(pane)?;
    // Hub clamps at 5 % anyway; matching the range here keeps the knob
    // position honest.
    bl_slider.set_range(5, 100).set_value(init_pct);
    bl_slider.width(lv_pct(85)).height(16);
    let bl_label = Label::new(pane)?;
    bl_label
      .text(&format!("{init_pct}%"))
      .add_style(&theme.body, Selector::DEFAULT);

    let table = Table::new(pane)?;
    table.set_column_count(2).set_row_count(8);
    table.set_column_width(0, 120).set_column_width(1, 168);
    table.width(lv_pct(100));
    let table_style = oxivgl::style::Style::new(|s| {
      s.bg_color_hex(crate::ui::C_CARD)
        .text_color_hex(crate::ui::C_TEXT)
        .text_font(oxivgl::fonts::MONTSERRAT_14)
        .border_width(0)
        .radius(10);
    });
    table.add_style(&table_style, Selector::DEFAULT);
    for (row, name) in [
      (ROW_IP, "IP"),
      (ROW_WIFI, "WiFi"),
      (ROW_LVGL_USED, "LVGL used"),
      (ROW_LVGL_FREE, "LVGL free"),
      (ROW_HEAP_USED, "Heap used"),
      (ROW_HEAP_FREE, "Heap free"),
      (ROW_UPTIME, "Uptime"),
      (ROW_CHIP, "Chip"),
    ] {
      table.set_cell_value(row, 0, name);
      table.set_cell_value(row, 1, "-");
    }
    table.set_cell_value(ROW_CHIP, 1, "ESP32-S3 @ 240 MHz");

    let hint = Label::new(pane)?;
    hint
      .text("stats refresh 1 Hz")
      .add_style(&theme.dim, Selector::DEFAULT);

    self.bl_slider = Some(bl_slider);
    self.bl_label = Some(bl_label);
    self.table = Some(table);
    self._statics = Some((caption, hint));
    self.last_bl = init_pct;
    Ok(())
  }

  pub fn update(&mut self, active: bool, tick_1hz: bool) {
    if !active {
      return;
    }

    if let (Some(s), Some(l)) = (&self.bl_slider, &self.bl_label) {
      let v = s.get_value();
      if v != self.last_bl {
        self.last_bl = v;
        BACKLIGHT_PCT.store(v as u8, Ordering::Relaxed);
        l.text(&format!("{v}%"));
      }
    }

    if !tick_1hz {
      return;
    }
    if let Some(t) = &self.table {
      let wifi_state = WIFI_STATE.load(Ordering::Relaxed);
      let ip = WIFI_IP.load(Ordering::Relaxed);
      if ip != 0 {
        let [a, b, c, d] = ip.to_be_bytes();
        t.set_cell_value(ROW_IP, 1, &format!("{a}.{b}.{c}.{d}"));
      } else {
        t.set_cell_value(ROW_IP, 1, "-");
      }
      t.set_cell_value(ROW_WIFI, 1, wifi_text(wifi_state, ip).trim_start_matches("WiFi: "));

      // SAFETY: on the LVGL task; lv_mem_monitor fills the out-struct.
      let lv = unsafe {
        let mut mon = core::mem::MaybeUninit::<oxivgl_sys::lv_mem_monitor_t>::uninit();
        oxivgl_sys::lv_mem_monitor(mon.as_mut_ptr());
        mon.assume_init()
      };
      t.set_cell_value(ROW_LVGL_USED, 1, &format!("{} B", lv.total_size - lv.free_size));
      t.set_cell_value(ROW_LVGL_FREE, 1, &format!("{} B", lv.free_size));
      t.set_cell_value(ROW_HEAP_USED, 1, &format!("{} B", esp_alloc::HEAP.used()));
      t.set_cell_value(ROW_HEAP_FREE, 1, &format!("{} B", esp_alloc::HEAP.free()));

      let secs = embassy_time::Instant::now().as_secs();
      let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
      t.set_cell_value(ROW_UPTIME, 1, &format!("{h}:{m:02}:{s:02}"));
    }
  }
}
