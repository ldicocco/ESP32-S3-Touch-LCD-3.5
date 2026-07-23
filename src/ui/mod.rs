//! Demo UI: a four-tab LVGL app (`Home | IMU | Play | System`) sized for
//! the 320×480 portrait panel. The tab bar sits at the bottom (thumb
//! reach; the FPS overlay owns the top-left corner). Each tab is a
//! scrollable flex column of "cards".
//!
//! Data flows in through atomics published by the firmware tasks
//! (`sensors`, `wifi`, `touch`); `update()` runs every ~32 ms and diffs
//! each source against a cached value so widgets are only rewritten on
//! change. The UI talks back through `sensors::BACKLIGHT_PCT`.
//!
//! Styling: one shared [`Theme`] of `Style`s built in `create()` — the
//! per-object inline setters are deprecated in oxivgl (each call burns a
//! local style in the LVGL pool).
//!
//! Note: no color-format override here — oxivgl's default
//! `RGB565_SWAPPED` is exactly the ST7796's SPI wire order (`display.rs`).

mod home;
mod imu;
mod play;
mod system;

use alloc::format;
use alloc::string::String;

use oxivgl::fonts;
use oxivgl::indev::PointerIndev;
use oxivgl::style::{Selector, Style};
use oxivgl::timer::Timer;
use oxivgl::view::{NavAction, View};
use oxivgl::widgets::{DdDir, Obj, Tabview, WidgetError};

use crate::touch::TOUCH_STATE;
use crate::wifi::{STATE_CONNECTED, STATE_CONNECTING};

// Shared dark theme palette.
pub(crate) const C_CARD: u32 = 0x1c2536;
pub(crate) const C_TEXT: u32 = 0xe0e6f0;
pub(crate) const C_DIM: u32 = 0x8a93a6;
pub(crate) const C_ACCENT: u32 = 0x3b82f6;
pub(crate) const C_TRACK: u32 = 0x2a3348;
const C_BG: u32 = 0x101828;

const TAB_HOME: u32 = 0;
const TAB_IMU: u32 = 1;
const TAB_PLAY: u32 = 2;
const TAB_SYS: u32 = 3;

/// Shared styles, built once and applied via `add_style` (Style is
/// Rc-backed — every application just clones the handle).
pub(crate) struct Theme {
  /// Tab pane: padded flex-column container.
  pub pane: Style,
  /// Card surface: rounded, padded, full row width.
  pub card: Style,
  /// Transparent grouping container (flex rows).
  pub clear: Style,
  pub body: Style,
  pub dim: Style,
  pub title: Style,
}

impl Theme {
  fn new() -> Self {
    Self {
      pane: Style::new(|s| {
        s.pad_all(8).pad_row(10);
      }),
      card: Style::new(|s| {
        s.bg_color_hex(C_CARD)
          .bg_opa(255)
          .radius(10)
          .border_width(0)
          .pad_all(10);
      }),
      clear: Style::new(|s| {
        s.bg_opa(0).border_width(0).pad_all(0);
      }),
      body: Style::new(|s| {
        s.text_color_hex(C_TEXT).text_font(fonts::MONTSERRAT_14);
      }),
      dim: Style::new(|s| {
        s.text_color_hex(C_DIM).text_font(fonts::MONTSERRAT_12);
      }),
      title: Style::new(|s| {
        s.text_color_hex(C_TEXT).text_font(fonts::MONTSERRAT_16);
      }),
    }
  }
}

pub(crate) fn wifi_text(state: u8, ip: u32) -> String {
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

#[derive(Default)]
pub struct DemoView {
  /// LVGL pointer input device; registered on first `create` and kept for
  /// the life of the UI (dropping it would unregister the device).
  pointer: Option<PointerIndev>,
  tabs: Option<Tabview<'static>>,
  home: home::HomeTab,
  imu: imu::ImuTab,
  play: play::PlayTab,
  system: system::SystemTab,
  /// 1 Hz pacing for uptime / stats-table refreshes.
  slow: Option<Timer>,
}

impl View for DemoView {
  fn create(&mut self, container: &Obj<'static>) -> Result<(), WidgetError> {
    if self.pointer.is_none() {
      self.pointer = Some(PointerIndev::new(&TOUCH_STATE)?);
    }

    let theme = Theme::new();

    let screen_style = Style::new(|s| {
      s.bg_color_hex(C_BG).bg_opa(255).radius(0);
    });
    container.add_style(&screen_style, Selector::DEFAULT);
    container.remove_scrollable();

    let tabs = Tabview::new(container)?;
    tabs.set_tab_bar_position(DdDir::Bottom).set_tab_bar_size(56);
    tabs.add_style(&screen_style, Selector::DEFAULT);
    let bar_style = Style::new(|s| {
      s.bg_color_hex(C_CARD)
        .text_color_hex(C_TEXT)
        .text_font(fonts::MONTSERRAT_16);
    });
    tabs.get_tab_bar().add_style(&bar_style, Selector::DEFAULT);

    self.home.create(&tabs.add_tab("Home"), &theme)?;
    self.imu.create(&tabs.add_tab("IMU"), &theme)?;
    self.play.create(&tabs.add_tab("Play"), &theme)?;
    self.system.create(&tabs.add_tab("System"), &theme)?;
    self.tabs = Some(tabs);

    self.slow = Some(Timer::new(1000)?);

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
    let active = self.tabs.as_ref().map(|t| t.get_tab_active()).unwrap_or(0);
    // Consume the 1 Hz trigger exactly once per cycle and fan the bool out.
    let tick_1hz = self.slow.as_ref().is_some_and(|t| t.triggered());

    self.home.update(active == TAB_HOME, tick_1hz);
    self.imu.update(active == TAB_IMU);
    self.play.update(active == TAB_PLAY);
    self.system.update(active == TAB_SYS, tick_1hz);
    Ok(NavAction::None)
  }
}
