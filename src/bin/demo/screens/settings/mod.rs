//! Display brightness.

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

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/settings/settings.kdl");
}

const NODES: usize = 16;
const _: () = assert!(generated::SettingsApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::SettingsApp::HEIGHT == layout::CONTENT_SIZE.height);

pub(crate) struct SettingsScreen {
    backlight: Backlight,
    brightness: Brightness,
    gui: &'static mut gui::Context<NODES>,
    value_label: WidgetId,
    slider: Slider,
    dirty: bool,
}

impl SettingsScreen {
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
