//! Proximity and ambient light, from the one LTR-553 sensor behind the front
//! glass.
//!
//! The screen shows three values and two bars:
//!
//! - how close something is, in percent, with a bar,
//! - the raw count of the proximity sensor,
//! - the ambient light in lux, with a bar on a logarithmic scale, because
//!   the values go from 0 in the dark to thousands in sunlight.
//!
//! The labels come from the KDL layout file `proximity.kdl`. The value labels
//! and the bars are added in code, like on the settings screen. The screen
//! redraws only when a shown number changes. Without the sensor, it shows a
//! message.

use embassy_time::Instant;
use embedded_graphics::{
    prelude::{Dimensions as _, Size},
    primitives::Rectangle,
};
use embedded_gui::WidgetId;
use hack_and_hike::{
    capabilities::{
        display::Surface,
        light::Light,
        proximity::{self, Proximity},
    },
    ui::{Canvas, common, gui, theme},
};

use crate::{layout, screens::Screen, styles};

// The layout file becomes Rust code at compile time: a `...App` struct with a
// `build` function and one `WidgetId` for each named node.
/// The widgets generated from `proximity.kdl`: the labels and the slots for the
/// value labels and the bars.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/proximity/proximity.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// three value labels added in code.
const NODES: usize = 16;
const _: () = assert!(generated::ProximityApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::ProximityApp::HEIGHT == layout::CONTENT_SIZE.height);

/// The light bar is full at this many lux. A room with the lights on fills
/// about half of it, a flashlight pointed at the board fills all of it.
const FULL_BAR_LUX: f32 = 10_000.0;

/// The values as the screen shows them: whole numbers. The screen redraws
/// only when one of them changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shown {
    /// Ambient light in whole lux, with the fraction cut off.
    lux: i32,
    /// Closeness in percent, 0 to 100.
    percent: u8,
    /// The raw count of the proximity sensor.
    raw: u16,
}

/// Both handles of the sensor. The chip serves both capabilities, so the
/// board has either both or neither.
struct Sensor {
    /// Ambient light samples.
    light: Light,
    /// Proximity samples.
    proximity: Proximity,
}

/// The proximity screen and its state.
pub(crate) struct ProximityScreen {
    /// `None` when no sensor answered at boot.
    sensor: Option<Sensor>,
    /// The newest light value, in whole lux. `None` until the first sample.
    lux: Option<i32>,
    /// The newest proximity sample. `None` until the first sample.
    proximity: Option<proximity::Sample>,
    /// The values drawn last. `None` when the screen must be drawn again, for
    /// example after `enter`.
    shown: Option<Shown>,
    /// The widget tree built from `proximity.kdl`, plus the value labels that the
    /// code adds.
    gui: &'static mut gui::Context<NODES>,
    /// The "LIGHT LUX" readout.
    lux_label: WidgetId,
    /// The "PROXIMITY %" readout.
    percent_label: WidgetId,
    /// The "RAW COUNT" readout.
    raw_label: WidgetId,
    /// Where the light bar is drawn.
    lux_bar: Rectangle,
    /// Where the proximity bar is drawn.
    proximity_bar: Rectangle,
    /// Whether the "no sensor" message must be drawn.
    message_dirty: bool,
}

impl ProximityScreen {
    /// Build the layout and the value labels. `light` and `proximity` are
    /// `None` when the board has no sensor.
    pub(crate) fn new(light: Option<Light>, proximity: Option<Proximity>) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
        let app =
            generated::ProximityApp::build(gui).expect("proximity.kdl fits the GUI capacities");
        let lux_label =
            gui::add_value_label(gui, app.widgets.lux_value, "LIGHT LUX", 0, styles::value());
        let percent_label = gui::add_value_label(
            gui,
            app.widgets.proximity_value,
            "PROXIMITY %",
            0,
            styles::value(),
        );
        let raw_label =
            gui::add_value_label(gui, app.widgets.raw_value, "RAW COUNT", 0, styles::value());

        Self {
            sensor: light
                .zip(proximity)
                .map(|(light, proximity)| Sensor { light, proximity }),
            lux: None,
            proximity: None,
            shown: None,
            lux_bar: gui::slot(gui, app.widgets.lux_bar),
            proximity_bar: gui::slot(gui, app.widgets.proximity_bar),
            gui,
            lux_label,
            percent_label,
            raw_label,
            message_dirty: true,
        }
    }
}

impl Screen for ProximityScreen {
    fn enter(&mut self) {
        self.shown = None;
        self.message_dirty = true;
    }

    /// Keep the newest sample of each handle, also while the screen is
    /// hidden. Then the screen shows current values at once when it becomes
    /// visible.
    fn update(&mut self, _now: Instant) {
        let Some(sensor) = &mut self.sensor else {
            return;
        };
        if let Some(sample) = sensor.light.latest() {
            // `as` saturates: a very large value becomes `i32::MAX`.
            self.lux = Some(sample.lux as i32);
        }
        if let Some(sample) = sensor.proximity.latest() {
            self.proximity = Some(sample);
        }
    }

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        if self.sensor.is_none() {
            if self.message_dirty {
                self.message_dirty = false;
                canvas.clear(theme::WHITE);
                common::centered_text(
                    canvas,
                    canvas.bounding_box(),
                    "No light sensor detected",
                    common::BODY_FONT,
                    theme::DARK_GRAY,
                );
                canvas.show(surface);
            }
            return;
        }

        // Draw when both handles have delivered a sample. Both come about
        // every 100 ms, so this waits only briefly after boot.
        let (Some(lux), Some(proximity)) = (self.lux, self.proximity) else {
            return;
        };
        let shown = Shown {
            lux,
            percent: proximity.percent,
            raw: proximity.raw,
        };
        if self.shown == Some(shown) {
            return;
        }
        self.shown = Some(shown);

        gui::set_value(self.gui, self.lux_label, shown.lux);
        gui::set_value(self.gui, self.percent_label, i32::from(shown.percent));
        gui::set_value(self.gui, self.raw_label, i32::from(shown.raw));

        canvas.clear(theme::WHITE);
        gui::render(self.gui, canvas);
        draw_bar(canvas, self.lux_bar, lux_fraction(shown.lux));
        draw_bar(canvas, self.proximity_bar, f32::from(shown.percent) / 100.0);
        canvas.show(surface);
    }
}

/// How much of the light bar `lux` fills, 0.0 to 1.0, on a logarithmic
/// scale: each ten times more light adds the same length.
fn lux_fraction(lux: i32) -> f32 {
    let lux = lux.max(0) as f32;
    (libm::log10f(1.0 + lux) / libm::log10f(1.0 + FULL_BAR_LUX)).clamp(0.0, 1.0)
}

/// Draw a bar in `area`: a light gray track, filled from the left by
/// `fraction` (0.0 to 1.0) in dark blue.
fn draw_bar(canvas: &mut Canvas, area: Rectangle, fraction: f32) {
    canvas.fill(area, theme::LIGHT_GRAY);
    let filled = (area.size.width as f32 * fraction.clamp(0.0, 1.0)) as u32;
    canvas.fill(
        Rectangle::new(area.top_left, Size::new(filled, area.size.height)),
        theme::DARK_BLUE,
    );
}
