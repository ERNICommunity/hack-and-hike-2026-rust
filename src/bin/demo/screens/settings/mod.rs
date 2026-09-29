//! Display brightness: a slider that sets the backlight, saved in flash.
//!
//! Copy this screen when you add a screen. It has:
//!
//! - a KDL layout file, `settings.kdl`, for the static labels (KDL is a small
//!   document language),
//! - a value label and a slider, added in code,
//! - one capability handle, and the storage to keep its setting.
//!
//! It redraws only when the brightness changed.
//!
//! The brightness survives a restart. At start-up, the screen loads it from
//! the storage and sets the backlight. It saves the brightness when the
//! finger leaves the slider, and only when it changed. A save takes about
//! half a second, so the screen does not save during a drag. It also waits
//! until the new value is on the screen.

use embassy_time::Instant;
use embedded_gui::WidgetId;
use hack_and_hike::{
    capabilities::{
        backlight::{Backlight, Brightness},
        display::Surface,
        storage::{Storage, StorageError},
        touch::TouchEvent,
    },
    ui::{Canvas, gui, theme, widgets::Slider},
};

use crate::{layout, screens::Screen, styles};

// The layout file becomes Rust code at compile time: a `...App` struct with a
// `build` function and one `WidgetId` for each named node.
/// The widgets generated from `settings.kdl`: the labels and the slots for
/// the value label and the slider.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/settings/settings.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// widgets added in code.
const NODES: usize = 16;
const _: () = assert!(generated::SettingsApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::SettingsApp::HEIGHT == layout::CONTENT_SIZE.height);

/// The name of the saved record. Its data is one byte: the brightness in
/// percent. Change the version when the data changes its meaning.
const RECORD: &str = "demo/settings/v1";

/// The settings screen and its state.
pub(crate) struct SettingsScreen {
    /// The handle that sets the LCD backlight level.
    backlight: Backlight,
    /// The brightness last requested.
    brightness: Brightness,
    /// Keeps the brightness through a restart. `None` when the board has no
    /// storage; then the setting is lost at a restart.
    storage: Option<Storage>,
    /// The brightness in the storage, or the start-up brightness when
    /// nothing is saved.
    saved: Brightness,
    /// Whether the finger left the slider since the last save. Then the
    /// brightness is saved once the screen shows it.
    save_pending: bool,
    /// The widget tree built from `settings.kdl`, plus the value label that the
    /// code adds.
    gui: &'static mut gui::Context<NODES>,
    /// The "BRIGHTNESS %" readout.
    value_label: WidgetId,
    /// Turns touches into a brightness percentage and draws itself. It is
    /// drawn on top of the GUI and is not part of it.
    slider: Slider,
    /// Whether the screen needs a redraw.
    dirty: bool,
}

impl SettingsScreen {
    /// Build the layout and the widgets. Set the backlight to the saved
    /// brightness, or leave it at full brightness when nothing is saved.
    pub(crate) fn new(mut backlight: Backlight, mut storage: Option<Storage>) -> Self {
        let brightness = storage
            .as_mut()
            .and_then(load_brightness)
            .unwrap_or(Brightness::FULL);
        if brightness != Brightness::FULL {
            backlight.set(brightness);
        }
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
        let app = generated::SettingsApp::build(gui).expect("settings.kdl fits the GUI capacities");
        let value_label = gui::add_value_label(
            gui,
            app.widgets.brightness_value,
            "BRIGHTNESS %",
            i32::from(brightness.percent()),
            styles::value(),
        );
        let slider = Slider::new(
            gui::slot(gui, app.widgets.brightness_slider),
            i32::from(Brightness::MIN.percent()),
            i32::from(Brightness::FULL.percent()),
        );

        Self {
            backlight,
            brightness,
            storage,
            saved: brightness,
            save_pending: false,
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

    /// The screen is not drawn while hidden, and `enter` redraws it. So a
    /// pending save need not wait for the next visit.
    fn leave(&mut self) {
        self.dirty = false;
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
        if let TouchEvent::Released(_) = event {
            self.save_pending = true;
        }
    }

    /// Save after `present` showed the new brightness, so the save does not
    /// delay the picture.
    fn update(&mut self, _now: Instant) {
        if !self.save_pending || self.dirty {
            return;
        }
        self.save_pending = false;
        if self.brightness == self.saved {
            return;
        }
        let Some(storage) = self.storage.as_mut() else {
            return;
        };
        match storage.save(RECORD, &[self.brightness.percent()]) {
            Ok(()) => self.saved = self.brightness,
            Err(error) => log::warn!("Brightness not saved: {error:?}"),
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

/// The saved brightness, or `None` when nothing valid is saved.
fn load_brightness(storage: &mut Storage) -> Option<Brightness> {
    let mut buffer = [0; 1];
    match storage.load(RECORD, &mut buffer) {
        Ok(&[percent]) => Brightness::try_from(percent).ok(),
        // Nothing saved yet, or another application saved its record.
        Err(StorageError::Empty | StorageError::OtherRecord) => None,
        other => {
            log::warn!("The saved brightness is not valid: {other:?}");
            None
        }
    }
}
