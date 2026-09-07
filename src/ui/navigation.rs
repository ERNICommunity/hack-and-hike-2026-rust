//! Fixed navigation rail input and rendering.
//!
//! Navigation is deliberately simple: five fixed 44×48 hit regions and five
//! compile-time bitmap icons. There is no widget tree, dynamic layout, or heap
//! activity in either input dispatch or rendering.

use crate::{
    display::Display,
    models::ViewId,
    theme,
    touch::{self, TouchPoint},
};

use super::layout;

const ICON_SIZE: usize = 16;
const ICON_X: usize = (layout::NAV_WIDTH - ICON_SIZE) / 2;
const ICON_Y_IN_BUTTON: usize = (layout::NAV_BUTTON_HEIGHT - ICON_SIZE) / 2;

const _: () = assert!(ViewId::ALL.len() * layout::NAV_BUTTON_HEIGHT == crate::display::HEIGHT);

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

#[derive(Clone, Copy)]
pub(crate) struct NavigationInput {
    pressed: bool,
    candidate: Option<ViewId>,
}

impl NavigationInput {
    pub(crate) const fn new() -> Self {
        Self {
            pressed: false,
            candidate: None,
        }
    }

    /// Drain all pending touch input and return a committed navigation target,
    /// if the gesture both started and ended inside the same navigation button.
    pub(crate) fn poll(&mut self) -> Option<ViewId> {
        let mut selected = None;

        while let Some(edge) = touch::try_take_edge() {
            match edge {
                touch::TouchEdge::Pressed(point) => {
                    self.pressed = true;
                    self.candidate = view_at(point);
                }
                touch::TouchEdge::Released(point) => {
                    if self.pressed && self.candidate == view_at(point) {
                        selected = self.candidate;
                    }
                    self.pressed = false;
                    self.candidate = None;
                }
            }
        }

        if self.pressed {
            if let Some(point) = touch::take_latest_point() {
                if view_at(point) != self.candidate {
                    self.candidate = None;
                }
            }
        } else {
            let _ = touch::take_latest_point();
        }

        selected
    }
}

fn view_at(point: TouchPoint) -> Option<ViewId> {
    if usize::from(point.x) >= layout::NAV_WIDTH {
        return None;
    }

    let index = usize::from(point.y) / layout::NAV_BUTTON_HEIGHT;
    ViewId::ALL.get(index).copied()
}

pub(crate) fn render(display: &mut Display, active: ViewId) {
    display.render_scanlines(layout::NAV_REGION, |screen_y, pixels| {
        let button_index = screen_y / layout::NAV_BUTTON_HEIGHT;
        let selected = ViewId::ALL[button_index] == active;
        let background = if selected {
            theme::LIGHT_BLUE_RGB565
        } else {
            theme::DARK_BLUE_RGB565
        };
        let foreground = if selected {
            theme::WHITE_RGB565
        } else {
            theme::DARK_GRAY_RGB565
        };

        pixels.fill(background);
        pixels[layout::NAV_WIDTH - 1] = theme::DARK_BLUE_RGB565;

        let local_y = screen_y % layout::NAV_BUTTON_HEIGHT;
        if (ICON_Y_IN_BUTTON..ICON_Y_IN_BUTTON + ICON_SIZE).contains(&local_y) {
            let row_bits = NAV_ICONS[button_index][local_y - ICON_Y_IN_BUTTON];
            for icon_x in 0..ICON_SIZE {
                if row_bits & (1 << (ICON_SIZE - 1 - icon_x)) != 0 {
                    pixels[ICON_X + icon_x] = foreground;
                }
            }
        }
    });
}
