//! Settings application state.

use crate::capabilities::display::{Brightness, BrightnessControl};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DisplayState {
    pub(super) brightness: Brightness,
}

pub(super) struct Model {
    control: BrightnessControl,
    display: DisplayState,
    dirty: bool,
}

impl Model {
    pub(super) fn new(control: BrightnessControl) -> Self {
        Self {
            control,
            display: DisplayState {
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

    pub(super) fn take_display(&mut self) -> Option<DisplayState> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}
