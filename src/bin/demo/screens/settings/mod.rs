//! Display brightness: a slider that sets the backlight.
//!
//! The screen to copy when you add one: a KDL layout (`settings.kdl`) for
//! the static labels, a value label and a slider added in code, and one
//! capability handle. It redraws only when the brightness changed.

use embedded_gui::WidgetId;
use hack_and_hike::{
    capabilities::{
        backlight::{Backlight, Brightness},
        display::Surface,
        touch::TouchEvent,
    },
    ui::{Canvas, gui, theme, widgets::Slider},
};

use crate::{layout, screens::Screen, styles};

// The layout file becomes Rust at compile time: a `...App` struct with a
// `build` function and one `WidgetId` per named node.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/settings/settings.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// widgets added in code.
const NODES: usize = 16;
const _: () = assert!(generated::SettingsApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::SettingsApp::HEIGHT == layout::CONTENT_SIZE.height);

/// The settings screen and its state.
pub(crate) struct SettingsScreen {
    backlight: Backlight,
    /// The brightness last requested.
    brightness: Brightness,
    gui: &'static mut gui::Context<NODES>,
    /// The "BRIGHTNESS %" readout.
    value_label: WidgetId,
    slider: Slider,
    /// Whether the screen needs a redraw.
    dirty: bool,
}

impl SettingsScreen {
    /// Build the layout and the widgets; the backlight starts at full
    /// brightness.
    pub(crate) fn new(backlight: Backlight) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
        let app = generated::SettingsApp::build(gui).expect("settings.kdl fits the GUI capacities");
        let value_label = gui::add_value_label(
            gui,
            app.widgets.brightness_value,
            "BRIGHTNESS %",
            i32::from(Brightness::FULL.percent()),
            styles::value(),
        );
        let slider = Slider::new(
            gui::slot(gui, app.widgets.brightness_slider),
            i32::from(Brightness::MIN.percent()),
            i32::from(Brightness::FULL.percent()),
        );

        Self {
            backlight,
            brightness: Brightness::FULL,
            gui,
            value_label,
            slider,
            dirty: true,
        }
    }
}

impl Screen for SettingsScreen {
    fn enter(&mut self) {
        self.dirty = true;
    }

    fn handle_touch(&mut self, event: TouchEvent) {
        let brightness = self
            .slider
            .handle_touch(event)
            .and_then(|percent| u8::try_from(percent).ok())
            .and_then(|percent| Brightness::try_from(percent).ok());
        if let Some(brightness) = brightness
            && brightness != self.brightness
        {
            self.brightness = brightness;
            self.backlight.set(brightness);
            self.dirty = true;
        }
    }

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let percent = i32::from(self.brightness.percent());
        gui::set_value(self.gui, self.value_label, percent);

        canvas.clear(theme::WHITE);
        gui::render(self.gui, canvas);
        self.slider.draw(canvas, percent);
        canvas.show(surface);
    }
}
