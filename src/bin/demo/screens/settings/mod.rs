//! Display brightness.

use embedded_gui::WidgetId;
use hack_and_hike::{
    capabilities::{
        backlight::{Backlight, Brightness},
        display::Surface,
    },
    ui::{
        gui::{self, GuiSurface, Pointer},
        widgets::Slider,
    },
};

use crate::{layout, screens::Screen, styles};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/settings/settings.kdl");
}

const NODES: usize = 16;

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
        let gui = gui::context::<NODES>(layout::CONTENT_WIDTH, layout::CONTENT_HEIGHT);
        let app = generated::SettingsApp::build(gui).expect("settings.kdl fits the GUI capacities");
        let value_label = gui
            .add_value_label(
                gui::slot(gui, app.widgets.brightness_value),
                "BRIGHTNESS %",
                i32::from(Brightness::FULL.percent()),
                styles::value(),
            )
            .expect("room for the brightness value");
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

    fn handle_pointer(&mut self, pointer: Pointer) {
        let Some(percent) = self.slider.handle_pointer(pointer) else {
            return;
        };
        let Some(brightness) = u8::try_from(percent).ok().and_then(Brightness::new) else {
            return;
        };
        if brightness != self.brightness {
            self.brightness = brightness;
            self.backlight.set(brightness);
            self.dirty = true;
        }
    }

    fn present(&mut self, gui: &mut GuiSurface, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let percent = i32::from(self.brightness.percent());
        gui::set_value(self.gui, self.value_label, percent);
        let slider = self.slider;
        gui.present(surface, self.gui, |frame| slider.draw(frame, percent));
    }
}
