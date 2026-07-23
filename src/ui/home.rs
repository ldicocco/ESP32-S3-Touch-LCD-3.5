//! Home tab: RTC clock, date, Wi-Fi status card, battery gauge card
//! (AXP2101), uptime.

use core::sync::atomic::Ordering;

use alloc::format;

use oxivgl::fonts;
use oxivgl::layout::{FlexAlign, FlexFlow};
use oxivgl::style::{LV_SIZE_CONTENT, Selector, Style, lv_pct};
use oxivgl::widgets::{Align, Arc, Label, Obj, WidgetError};

use crate::sensors::{
  BAT_MV, BAT_PERCENT, BAT_PERCENT_UNKNOWN, PMU_CHARGE_DONE, PMU_CHARGING, PMU_FLAGS, PMU_OK,
  PMU_VBUS, RTC_DATE, RTC_HMS, unpack_date, unpack_hms,
};
use crate::ui::{C_ACCENT, C_TEXT, C_TRACK, Theme, wifi_text};
use crate::wifi::{WIFI_IP, WIFI_STATE};

#[derive(Default)]
pub struct HomeTab {
  clock: Option<Label<'static>>,
  date: Option<Label<'static>>,
  wifi: Option<Label<'static>>,
  bat_arc: Option<Arc<'static>>,
  bat_pct: Option<Label<'static>>,
  bat_detail: Option<Label<'static>>,
  uptime: Option<Label<'static>>,
  _cards: Option<(Obj<'static>, Obj<'static>)>,
  last_hms: u32,
  last_date: u32,
  last_wifi: (u8, u32),
  last_bat: (u16, u8, u8),
  last_uptime_s: u64,
}

impl HomeTab {
  pub fn create(&mut self, pane: &Obj<'static>, theme: &Theme) -> Result<(), WidgetError> {
    pane.add_style(&theme.pane, Selector::DEFAULT);
    pane.set_flex_flow(FlexFlow::Column);
    pane.set_flex_align(FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);

    let clock_style = Style::new(|s| {
      s.text_color_hex(C_TEXT).text_font(fonts::MONTSERRAT_48);
    });
    let clock = Label::new(pane)?;
    clock.text("--:--:--").add_style(&clock_style, Selector::DEFAULT);

    let date = Label::new(pane)?;
    date.text("RTC not set").add_style(&theme.dim, Selector::DEFAULT);

    let wifi_card = Obj::new(pane)?;
    wifi_card
      .add_style(&theme.card, Selector::DEFAULT)
      .width(lv_pct(100))
      .height(LV_SIZE_CONTENT)
      .remove_scrollable();
    let wifi = Label::new(&wifi_card)?;
    wifi
      .text(wifi_text(WIFI_STATE.load(Ordering::Relaxed), 0).as_str())
      .add_style(&theme.body, Selector::DEFAULT)
      .center();

    let bat_card = Obj::new(pane)?;
    bat_card
      .add_style(&theme.card, Selector::DEFAULT)
      .width(lv_pct(100))
      .height(170)
      .remove_scrollable();
    let bat_arc = Arc::gauge_ring(&bat_card, 120, 10, 100.0, C_TRACK, C_ACCENT, 135, 270)?;
    let pct_style = Style::new(|s| {
      s.text_color_hex(C_TEXT).text_font(fonts::MONTSERRAT_20);
    });
    let bat_pct = Label::new(&bat_card)?;
    bat_pct
      .text("--")
      .add_style(&pct_style, Selector::DEFAULT)
      .align(Align::Center, 0, -8);
    let bat_detail = Label::new(&bat_card)?;
    bat_detail
      .text("PMU...")
      .add_style(&theme.dim, Selector::DEFAULT)
      .align(Align::BottomMid, 0, 0);

    let uptime = Label::new(pane)?;
    uptime.text("up 0s").add_style(&theme.dim, Selector::DEFAULT);

    self.clock = Some(clock);
    self.date = Some(date);
    self.wifi = Some(wifi);
    self.bat_arc = Some(bat_arc);
    self.bat_pct = Some(bat_pct);
    self.bat_detail = Some(bat_detail);
    self.uptime = Some(uptime);
    self._cards = Some((wifi_card, bat_card));
    Ok(())
  }

  pub fn update(&mut self, active: bool, tick_1hz: bool) {
    // Clock / date / wifi / battery sources change at ≤1 Hz, so diffing
    // them every frame is cheap and keeps them fresh on tab switch.
    let hms = RTC_HMS.load(Ordering::Relaxed);
    if hms != self.last_hms {
      self.last_hms = hms;
      if let Some(l) = &self.clock {
        match unpack_hms(hms) {
          Some((h, m, s)) => l.text(&format!("{h:02}:{m:02}:{s:02}")),
          None => l.text("--:--:--"),
        };
      }
    }

    let date = RTC_DATE.load(Ordering::Relaxed);
    if date != self.last_date {
      self.last_date = date;
      if let (Some(l), Some((y, mo, d))) = (&self.date, unpack_date(date)) {
        l.text(&format!("{y:04}-{mo:02}-{d:02}"));
      }
    }

    let wifi = (
      WIFI_STATE.load(Ordering::Relaxed),
      WIFI_IP.load(Ordering::Relaxed),
    );
    if wifi != self.last_wifi {
      self.last_wifi = wifi;
      if let Some(l) = &self.wifi {
        l.text(wifi_text(wifi.0, wifi.1).as_str());
      }
    }

    let bat = (
      BAT_MV.load(Ordering::Relaxed),
      BAT_PERCENT.load(Ordering::Relaxed),
      PMU_FLAGS.load(Ordering::Relaxed),
    );
    if bat != self.last_bat {
      self.last_bat = bat;
      self.update_battery(bat);
    }

    if active && tick_1hz {
      let secs = embassy_time::Instant::now().as_secs();
      if secs != self.last_uptime_s {
        self.last_uptime_s = secs;
        if let Some(l) = &self.uptime {
          let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
          l.text(&format!("up {h}:{m:02}:{s:02}"));
        }
      }
    }
  }

  fn update_battery(&self, (mv, pct, flags): (u16, u8, u8)) {
    let (Some(arc), Some(pl), Some(dl)) = (&self.bat_arc, &self.bat_pct, &self.bat_detail) else {
      return;
    };
    if flags & PMU_OK == 0 {
      arc.set_value(0.0);
      pl.text("--");
      dl.text("PMU unavailable");
    } else if pct == BAT_PERCENT_UNKNOWN {
      arc.set_value(0.0);
      pl.text("USB");
      dl.text(if flags & PMU_VBUS != 0 {
        "USB power, no battery"
      } else {
        "no battery"
      });
    } else {
      arc.set_value(pct as f32);
      pl.text(&format!("{pct}%"));
      let state = if flags & PMU_CHARGING != 0 {
        "charging"
      } else if flags & PMU_CHARGE_DONE != 0 {
        "charged"
      } else if flags & PMU_VBUS != 0 {
        "on USB"
      } else {
        "on battery"
      };
      dl.text(&format!("{mv} mV  {state}"));
    }
  }
}
