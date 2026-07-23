//! IMU tab: live 3-axis accelerometer chart (6 s of history), numeric
//! readouts, a bubble level driven by accel x/y, and a gyro line.

use core::sync::atomic::Ordering;

use alloc::format;

use oxivgl::fonts;
use oxivgl::layout::{FlexAlign, FlexFlow};
use oxivgl::style::{LV_SIZE_CONTENT, Palette, Selector, Style, lv_pct, palette_main};
use oxivgl::widgets::{
  Align, Chart, ChartAxis, ChartSeries, ChartType, ChartUpdateMode, Label, Obj, Part, WidgetError,
};

use crate::sensors::{IMU_ACCEL_MG, IMU_GYRO_MDPS, IMU_SEQ, IMU_STATE};
use crate::ui::{C_ACCENT, C_CARD, C_DIM, Theme};

/// Chart span: 60 points @ 10 Hz = 6 s of history.
const CHART_POINTS: u32 = 60;
/// Display window ±2 g in mg.
const CHART_RANGE_MG: i32 = 2000;
/// Bubble level geometry (px).
const LEVEL_SIZE: i32 = 130;
const BUBBLE_SIZE: i32 = 18;
/// Bubble travel: full deflection at 1 g.
const BUBBLE_TRAVEL: i32 = (LEVEL_SIZE - BUBBLE_SIZE) / 2 - 2;

/// Axis display colors (chart series use the palette equivalents).
const AXIS_HEX: [u32; 3] = [0xef5350, 0x66bb6a, 0x42a5f5];
const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

#[derive(Default)]
pub struct ImuTab {
  chart: Option<Chart<'static>>,
  series: Option<[ChartSeries; 3]>,
  readout: Option<[Label<'static>; 3]>,
  bubble: Option<Obj<'static>>,
  gyro: Option<Label<'static>>,
  _statics: Option<(Obj<'static>, Obj<'static>, Obj<'static>, Obj<'static>)>,
  last_seq: u32,
  absent_noted: bool,
}

impl ImuTab {
  pub fn create(&mut self, pane: &Obj<'static>, theme: &Theme) -> Result<(), WidgetError> {
    pane.add_style(&theme.pane, Selector::DEFAULT);
    pane.set_flex_flow(FlexFlow::Column);
    pane.set_flex_align(FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);

    let chart_style = Style::new(|s| {
      s.bg_color_hex(C_CARD).radius(10).border_width(0);
    });
    // Hide the per-point dots — a 60-point line chart is unreadable with
    // them at this size.
    let no_dots = Style::new(|s| {
      s.size(0, 0);
    });
    let chart = Chart::new(pane)?;
    chart.width(lv_pct(100)).height(160);
    chart.add_style(&chart_style, Selector::DEFAULT);
    chart.add_style(&no_dots, Part::Indicator);
    chart
      .set_type(ChartType::Line)
      .set_point_count(CHART_POINTS)
      .set_axis_range(ChartAxis::PrimaryY, -CHART_RANGE_MG, CHART_RANGE_MG)
      .set_update_mode(ChartUpdateMode::Shift)
      .set_div_line_count(5, 4);

    let palettes = [Palette::Red, Palette::Green, Palette::Blue];
    let series = palettes.map(|c| chart.add_series(palette_main(c), ChartAxis::PrimaryY));

    let row = Obj::new(pane)?;
    row
      .add_style(&theme.clear, Selector::DEFAULT)
      .width(lv_pct(100))
      .height(LV_SIZE_CONTENT)
      .remove_scrollable();
    row.set_flex_flow(FlexFlow::Row);
    row.set_flex_align(FlexAlign::SpaceEvenly, FlexAlign::Center, FlexAlign::Center);
    let mk_readout = |i: usize| -> Result<Label<'static>, WidgetError> {
      let axis_style = Style::new(|s| {
        s.text_color_hex(AXIS_HEX[i]).text_font(fonts::MONTSERRAT_14);
      });
      let l = Label::new(&row)?;
      l.text(&format!("{} --", AXIS_NAMES[i]))
        .add_style(&axis_style, Selector::DEFAULT);
      Ok(l)
    };
    let readout = [mk_readout(0)?, mk_readout(1)?, mk_readout(2)?];

    // Bubble level: a bordered circle with a floating dot, driven by
    // accel x/y (full deflection at 1 g).
    let level_card = Obj::new(pane)?;
    level_card
      .add_style(&theme.card, Selector::DEFAULT)
      .width(lv_pct(100))
      .height(LEVEL_SIZE + 20)
      .remove_scrollable();
    let level = Obj::new(&level_card)?;
    let level_style = Style::new(|s| {
      s.radius_circle()
        .bg_color_hex(C_CARD)
        .border_width(2)
        .border_color_hex(C_DIM);
    });
    level
      .width(LEVEL_SIZE)
      .height(LEVEL_SIZE)
      .add_style(&level_style, Selector::DEFAULT)
      .center()
      .remove_scrollable();
    let center_style = Style::new(|s| {
      s.radius_circle().bg_color_hex(C_DIM).border_width(0);
    });
    let center_dot = Obj::new(&level)?;
    center_dot
      .width(6)
      .height(6)
      .add_style(&center_style, Selector::DEFAULT)
      .align(Align::Center, 0, 0);
    let bubble_style = Style::new(|s| {
      s.radius_circle().bg_color_hex(C_ACCENT).border_width(0);
    });
    let bubble = Obj::new(&level)?;
    bubble
      .width(BUBBLE_SIZE)
      .height(BUBBLE_SIZE)
      .add_style(&bubble_style, Selector::DEFAULT)
      .align(Align::Center, 0, 0);

    let gyro = Label::new(pane)?;
    gyro.text("gyro: --").add_style(&theme.dim, Selector::DEFAULT);

    self.chart = Some(chart);
    self.series = Some(series);
    self.readout = Some(readout);
    self.bubble = Some(bubble);
    self.gyro = Some(gyro);
    self._statics = Some((row, level_card, level, center_dot));
    Ok(())
  }

  pub fn update(&mut self, active: bool) {
    if IMU_STATE.load(Ordering::Relaxed) == 0 {
      if !self.absent_noted {
        self.absent_noted = true;
        if let Some(l) = &self.gyro {
          l.text("IMU not detected");
        }
      }
      return;
    }

    let seq = IMU_SEQ.load(Ordering::Relaxed);
    if seq == self.last_seq {
      return;
    }
    self.last_seq = seq;

    let accel = [
      IMU_ACCEL_MG[0].load(Ordering::Relaxed),
      IMU_ACCEL_MG[1].load(Ordering::Relaxed),
      IMU_ACCEL_MG[2].load(Ordering::Relaxed),
    ];

    // Feed the chart even when the tab is hidden (µs cost) so the history
    // is continuous when the user switches back.
    if let (Some(chart), Some(series)) = (&self.chart, &self.series) {
      for (s, &v) in series.iter().zip(accel.iter()) {
        chart.set_next_value(s, v.clamp(-CHART_RANGE_MG, CHART_RANGE_MG));
      }
    }

    // Labels and bubble position are churn — visible tab only.
    if !active {
      return;
    }

    if let Some(readout) = &self.readout {
      for i in 0..3 {
        readout[i].text(&format!("{} {:+05}", AXIS_NAMES[i], accel[i]));
      }
    }

    if let Some(bubble) = &self.bubble {
      // A level shows where the *bubble* goes: opposite the tilt's low
      // side, i.e. against the gravity component in the panel plane.
      let dx = (-accel[0] * BUBBLE_TRAVEL / 1000).clamp(-BUBBLE_TRAVEL, BUBBLE_TRAVEL);
      let dy = (accel[1] * BUBBLE_TRAVEL / 1000).clamp(-BUBBLE_TRAVEL, BUBBLE_TRAVEL);
      bubble.align(Align::Center, dx, dy);
    }

    if let Some(l) = &self.gyro {
      let g = [
        IMU_GYRO_MDPS[0].load(Ordering::Relaxed) / 1000,
        IMU_GYRO_MDPS[1].load(Ordering::Relaxed) / 1000,
        IMU_GYRO_MDPS[2].load(Ordering::Relaxed) / 1000,
      ];
      l.text(&format!("gyro dps  x {:+} y {:+} z {:+}", g[0], g[1], g[2]));
    }
  }
}
