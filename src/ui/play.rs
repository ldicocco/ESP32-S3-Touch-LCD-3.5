//! Play tab: the interactive widget playground — tap counter, slider,
//! switch + LED, roller, live touch coordinates.

use core::sync::atomic::{AtomicU32, Ordering};

use alloc::format;

use oxivgl::enums::{EventCode, ObjState};
use oxivgl::layout::{FlexAlign, FlexFlow};
use oxivgl::style::{LV_SIZE_CONTENT, Palette, Selector, lv_pct, palette_main};
use oxivgl::widgets::{Button, Label, Led, Obj, Roller, RollerMode, Slider, Switch, WidgetError};

use crate::touch::{TOUCH_XY, unpack};
use crate::ui::Theme;

static TAPS: AtomicU32 = AtomicU32::new(0);

fn on_button_clicked(_event: &oxivgl::event::Event) {
  TAPS.fetch_add(1, Ordering::Relaxed);
}

const ROLLER_OPTIONS: &str = "Espressif\nWaveshare\nEmbassy\nLVGL\nRust";

#[derive(Default)]
pub struct PlayTab {
  taps_label: Option<Label<'static>>,
  slider: Option<Slider<'static>>,
  slider_label: Option<Label<'static>>,
  switch: Option<Switch<'static>>,
  led: Option<Led<'static>>,
  roller: Option<Roller<'static>>,
  roller_label: Option<Label<'static>>,
  coords: Option<Label<'static>>,
  _statics: Option<(Button<'static>, Label<'static>, Obj<'static>)>,
  last_taps: u32,
  last_slider: i32,
  last_switch: bool,
  last_roller: u32,
  last_xy: u32,
}

impl PlayTab {
  pub fn create(&mut self, pane: &Obj<'static>, theme: &Theme) -> Result<(), WidgetError> {
    pane.add_style(&theme.pane, Selector::DEFAULT);
    pane.set_flex_flow(FlexFlow::Column);
    pane.set_flex_align(FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);

    let button = Button::new(pane)?;
    button.width(lv_pct(100)).height(56);
    button.on(EventCode::CLICKED, on_button_clicked);
    let button_label = Label::new(&button)?;
    button_label
      .text("Tap me")
      .add_style(&theme.body, Selector::DEFAULT)
      .center();
    let taps = Label::new(pane)?;
    taps.text("Taps: 0").add_style(&theme.body, Selector::DEFAULT);

    let slider = Slider::new(pane)?;
    slider.set_range(0, 100).set_value(30);
    slider.width(lv_pct(85)).height(16);
    let slider_label = Label::new(pane)?;
    slider_label
      .text("Slider: 30")
      .add_style(&theme.body, Selector::DEFAULT);

    // Switch + LED row: the LED mirrors the switch state.
    let row = Obj::new(pane)?;
    row
      .add_style(&theme.card, Selector::DEFAULT)
      .width(lv_pct(100))
      .height(LV_SIZE_CONTENT)
      .remove_scrollable();
    row.set_flex_flow(FlexFlow::Row);
    row.set_flex_align(FlexAlign::SpaceEvenly, FlexAlign::Center, FlexAlign::Center);
    let switch = Switch::new(&row)?;
    let led = Led::new(&row)?;
    led.set_color(palette_main(Palette::Green)).off();

    let roller = Roller::new(pane)?;
    roller.set_options(ROLLER_OPTIONS, RollerMode::Infinite);
    roller.set_visible_row_count(3);
    let roller_label = Label::new(pane)?;
    roller_label
      .text("Selected: Espressif")
      .add_style(&theme.body, Selector::DEFAULT);

    let coords = Label::new(pane)?;
    coords.text("Touch: -").add_style(&theme.dim, Selector::DEFAULT);

    self.taps_label = Some(taps);
    self.slider = Some(slider);
    self.slider_label = Some(slider_label);
    self.switch = Some(switch);
    self.led = Some(led);
    self.roller = Some(roller);
    self.roller_label = Some(roller_label);
    self.coords = Some(coords);
    self._statics = Some((button, button_label, row));
    Ok(())
  }

  pub fn update(&mut self, active: bool) {
    let taps = TAPS.load(Ordering::Relaxed);
    if taps != self.last_taps {
      self.last_taps = taps;
      if let Some(l) = &self.taps_label {
        l.text(&format!("Taps: {taps}"));
      }
    }

    // Everything below can only change while the tab is visible.
    if !active {
      return;
    }

    if let (Some(s), Some(l)) = (&self.slider, &self.slider_label) {
      let v = s.get_value();
      if v != self.last_slider {
        self.last_slider = v;
        l.text(&format!("Slider: {v}"));
      }
    }

    if let (Some(sw), Some(led)) = (&self.switch, &self.led) {
      let on = sw.has_state(ObjState::CHECKED);
      if on != self.last_switch {
        self.last_switch = on;
        if on {
          led.on();
        } else {
          led.off();
        }
      }
    }

    if let (Some(r), Some(l)) = (&self.roller, &self.roller_label) {
      let sel = r.get_selected();
      if sel != self.last_roller {
        self.last_roller = sel;
        let mut buf = [0u8; 32];
        let name = r.get_selected_str(&mut buf).unwrap_or("?");
        l.text(&format!("Selected: {name}"));
      }
    }

    let xy = TOUCH_XY.load(Ordering::Relaxed);
    if xy != self.last_xy {
      self.last_xy = xy;
      if let Some(l) = &self.coords {
        match unpack(xy) {
          Some((x, y)) => l.text(&format!("Touch: {x}, {y}")),
          None => l.text("Touch: released"),
        };
      }
    }
  }
}
