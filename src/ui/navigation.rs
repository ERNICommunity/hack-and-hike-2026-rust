//! Fixed navigation rail input and rendering.
//!
//! `NavigationInput` owns the only CPU0 touch reader. Gesture state is separate
//! from `AppModel`: physical touch input becomes a semantic `ViewId` only after
//! a press and release complete inside the same navigation button.

use crate::{
    display::Display,
    models::ViewId,
    service_inputs::TouchInput,
    touch::{TouchEdge, TouchPoint},
};

use super::design;

const ICON_SIZE: usize = 16;
const ICON_X: usize = (design::UI.navigation.width - ICON_SIZE) / 2;
const ICON_Y_IN_BUTTON: usize = (design::UI.navigation.button_height - ICON_SIZE) / 2;

const _: () = assert!(ViewId::ALL.len() * design::UI.navigation.button_height == crate::display::HEIGHT);

const NAV_ICONS: [[u16; ICON_SIZE]; 5] = [
    [
        0x0000, 0x0000, 0x0180, 0x03C0, 0x0660, 0x0C30, 0x1818, 0x0180,
        0x0180, 0x1818, 0x0C30, 0x0660, 0x03C0, 0x0180, 0x0000, 0x0000,
    ],
    [
        0x0180, 0x0180, 0x0180, 0x0180, 0x0180, 0x7FFE, 0x0180, 0x0180,
        0x0180, 0x0180, 0x07E0, 0x0DB0, 0x198C, 0x0180, 0x0180, 0x0000,
    ],
    [
        0x03C0, 0x0660, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30,
        0x0660, 0x03C0, 0x0180, 0x1FF8, 0x0180, 0x0180, 0x07E0, 0x0000,
    ],
    [
        0x0000, 0x0300, 0x0700, 0x0F18, 0x7F0C, 0x7F06, 0x7F06, 0x7F06,
        0x7F06, 0x7F06, 0x7F0C, 0x0F18, 0x0700, 0x0300, 0x0000, 0x0000,
    ],
    [
        0x0000, 0x0000, 0x3FFC, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004,
        0x2FF4, 0x2004, 0x2FF4, 0x2004, 0x3FFC, 0x0000, 0x0000, 0x0000,
    ],
];

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
    ViewId::ALL.get(index).copied()
}

pub(crate) fn render(display: &mut Display, active: ViewId) {
    let nav = design::UI.navigation;
    display.render_scanlines(design::NAV_REGION, |screen_y, pixels| {
        let button_index = screen_y / nav.button_height;
        let selected = ViewId::ALL[button_index] == active;
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
        if (ICON_Y_IN_BUTTON..ICON_Y_IN_BUTTON + ICON_SIZE).contains(&local_y) {
            let row_bits = NAV_ICONS[button_index][local_y - ICON_Y_IN_BUTTON];
            for icon_x in 0..ICON_SIZE {
                if row_bits & (1 << (ICON_SIZE - 1 - icon_x)) != 0 {
                    pixels[ICON_X + icon_x] = foreground.raw();
                }
            }
        }
    });
}
