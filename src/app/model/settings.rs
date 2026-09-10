//! Settings presentation model.

use crate::services::display::{BrightnessControl, BrightnessPercent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SettingsDisplay {
    pub(crate) brightness: BrightnessPercent,
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
                brightness: BrightnessPercent::FULL,
            },
            dirty: true,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(super) fn set_brightness(&mut self, brightness: BrightnessPercent) {
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
