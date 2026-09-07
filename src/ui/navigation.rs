//! Fixed navigation rail input and rendering.
//!
//! `NavigationInput` owns the CPU0 touch reader used for presentation gestures.
//! Gesture state is separate from `AppModel`: physical touch input becomes a
//! semantic `ViewId` only after a press and release complete inside the same
//! navigation button. Hit testing and rendering consume the same declarative
//! navigation-item array from `ui::design`.

use crate::{
    display::Display,
    models::ViewId,
    service_inputs::TouchInput,
    touch::{TouchEdge, TouchPoint},
};

use super::design;

const ICON_X: usize = (design::UI.navigation.width - design::NAV_ICON_SIZE) / 2;
const ICON_Y_IN_BUTTON: usize =
    (design::UI.navigation.button_height - design::NAV_ICON_SIZE) / 2;

pub(crate) struct NavigationInput {
    touch: TouchInput,
    pressed: bool,
    candidate: Option<ViewId>,
}

impl NavigationInput {
    pub(crate) const fn new(touch: TouchInput) -> Self {
        Self {
            touch,
            pressed: false,
            candidate: None,
        }
    }

    /// Drain pending touch state and return a committed destination, if any.
    pub(crate) fn poll(&mut self) -> Option<ViewId> {
        let mut selected = None;

        while let Some(edge) = self.touch.next_edge() {
            match edge {
                TouchEdge::Pressed(point) => {
                    self.pressed = true;
                    self.candidate = view_at(point);
                }
                TouchEdge::Released(point) => {
                    if self.pressed && self.candidate == view_at(point) {
                        selected = self.candidate;
                    }
                    self.pressed = false;
                    self.candidate = None;
                }
            }
        }

        if self.pressed {
            if let Some(point) = self.touch.take_latest_point() {
                if view_at(point) != self.candidate {
                    self.candidate = None;
                }
            }
        } else {
            let _ = self.touch.take_latest_point();
        }

        selected
    }
}

fn view_at(point: TouchPoint) -> Option<ViewId> {
    let nav = design::UI.navigation;
    if usize::from(point.x) >= nav.width {
        return None;
    }

    let index = usize::from(point.y) / nav.button_height;
    nav.items.get(index).map(|item| item.view)
}

pub(crate) fn render(display: &mut Display, active: ViewId) {
    let nav = design::UI.navigation;
    display.render_scanlines(design::NAV_REGION, |screen_y, pixels| {
        let button_index = screen_y / nav.button_height;
        let item = &nav.items[button_index];
        let selected = item.view == active;
        let background = if selected {
            nav.selected_background
        } else {
            nav.normal_background
        };
        let foreground = if selected {
            nav.selected_icon
        } else {
            nav.normal_icon
        };

        pixels.fill(background.raw());
        pixels[nav.width - 1] = nav.divider.raw();

        let local_y = screen_y % nav.button_height;
        if (ICON_Y_IN_BUTTON..ICON_Y_IN_BUTTON + design::NAV_ICON_SIZE).contains(&local_y) {
            let row_bits = item.icon[local_y - ICON_Y_IN_BUTTON];
            for icon_x in 0..design::NAV_ICON_SIZE {
                if row_bits & (1 << (design::NAV_ICON_SIZE - 1 - icon_x)) != 0 {
                    pixels[ICON_X + icon_x] = foreground.raw();
                }
            }
        }
    });
}
