//! Settings presentation model.

use crate::capabilities::display::{Brightness, BrightnessControl};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SettingsDisplay {
    pub(crate) brightness: Brightness,
}

pub(super) struct Model {
    control: BrightnessControl,
    display: SettingsDisplay,
    dirty: bool,
}

impl Model {
    pub(super) fn new(control: BrightnessControl) -> Self {
        Self {
            control,
            display: SettingsDisplay {
                brightness: Brightness::FULL,
            },
            dirty: true,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(super) fn set_brightness(&mut self, brightness: Brightness) {
        if brightness == self.display.brightness {
            return;
        }
        self.display.brightness = brightness;
        self.control.set(brightness);
        self.dirty = true;
    }

    pub(super) fn take_display(&mut self) -> Option<SettingsDisplay> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}
