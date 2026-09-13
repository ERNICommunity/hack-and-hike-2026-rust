//! The navigation rail on the left and the routing of touches.
//!
//! A touch that starts on the rail selects a screen when it is released on
//! the same button. A touch that starts in the content area is forwarded to
//! the visible screen as a [`Pointer`].

use hack_and_hike::{
    capabilities::{
        display::Surface,
        touch::{Touch, TouchEdge, TouchPoint},
    },
    ui::{
        gui::{Pointer, PointerPhase},
        theme::pixel,
    },
};

use crate::layout::NAV_WIDTH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewId {
    Network,
    Imu,
    Microphone,
    Speaker,
    Camera,
    Settings,
    Log,
}

impl ViewId {
    /// Top-to-bottom order on the rail; the first one is shown at boot.
    pub(crate) const ALL: [Self; 7] = [
        Self::Network,
        Self::Imu,
        Self::Microphone,
        Self::Speaker,
        Self::Camera,
        Self::Settings,
        Self::Log,
    ];

    const fn icon(self) -> &'static Icon {
        match self {
            Self::Network => &NETWORK_ICON,
            Self::Imu => &IMU_ICON,
            Self::Microphone => &MICROPHONE_ICON,
            Self::Speaker => &SPEAKER_ICON,
            Self::Camera => &CAMERA_ICON,
            Self::Settings => &SETTINGS_ICON,
            Self::Log => &LOG_ICON,
        }
    }
}

const ICON_SIZE: usize = 16;
/// One row per line, most significant bit on the left.
type Icon = [u16; ICON_SIZE];

const BUTTON_HEIGHT: usize = hack_and_hike::capabilities::display::HEIGHT / ViewId::ALL.len();
const ICON_X: usize = (NAV_WIDTH - ICON_SIZE) / 2;
const ICON_Y: usize = (BUTTON_HEIGHT - ICON_SIZE) / 2;
const _: () = assert!(BUTTON_HEIGHT > ICON_SIZE);

const NETWORK_ICON: Icon = [
    0x0000, 0x0000, 0x0180, 0x03C0, 0x0660, 0x0C30, 0x1818, 0x0180, 0x0180, 0x1818, 0x0C30, 0x0660,
    0x03C0, 0x0180, 0x0000, 0x0000,
];
const IMU_ICON: Icon = [
    0x0180, 0x0180, 0x0180, 0x0180, 0x0180, 0x7FFE, 0x0180, 0x0180, 0x0180, 0x0180, 0x07E0, 0x0DB0,
    0x198C, 0x0180, 0x0180, 0x0000,
];
const MICROPHONE_ICON: Icon = [
    0x03C0, 0x0660, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0660, 0x03C0, 0x0180, 0x1FF8,
    0x0180, 0x0180, 0x07E0, 0x0000,
];
const SPEAKER_ICON: Icon = [
    0x0000, 0x0300, 0x0700, 0x0F18, 0x7F0C, 0x7F06, 0x7F06, 0x7F06, 0x7F06, 0x7F06, 0x7F0C, 0x0F18,
    0x0700, 0x0300, 0x0000, 0x0000,
];
const CAMERA_ICON: Icon = [
    0x0000, 0x0000, 0x0F00, 0x1980, 0x7FFE, 0x4002, 0x43C2, 0x4662, 0x4C32, 0x4C32, 0x4662, 0x43C2,
    0x4002, 0x7FFE, 0x0000, 0x0000,
];
const SETTINGS_ICON: Icon = [
    0x0000, 0x0180, 0x0DB0, 0x1FF8, 0x319C, 0x6186, 0x6786, 0x6606, 0x6606, 0x6786, 0x6186, 0x319C,
    0x1FF8, 0x0DB0, 0x0180, 0x0000,
];
const LOG_ICON: Icon = [
    0x0000, 0x0000, 0x3FFC, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004,
    0x3FFC, 0x0000, 0x0000, 0x0000,
];

/// Which part of the display a touch started on.
#[derive(Clone, Copy)]
enum Gesture {
    /// Started on a rail button; `None` once the finger left that button.
    Rail(Option<ViewId>),
    Content,
}

/// Owns the touch handle and turns raw touches into screen selections and
/// content pointers.
pub(crate) struct Navigation {
    touch: Touch,
    gesture: Option<Gesture>,
}

impl Navigation {
    pub(crate) const fn new(touch: Touch) -> Self {
        Self {
            touch,
            gesture: None,
        }
    }

    /// Process all pending touches. Content touches go to `on_content`; the
    /// return value is a screen the user selected on the rail, if any.
    pub(crate) fn poll(&mut self, mut on_content: impl FnMut(Pointer)) -> Option<ViewId> {
        let mut selected = None;

        while let Some(edge) = self.touch.next_edge() {
            self.forward_movement(&mut on_content);
            match edge {
                TouchEdge::Pressed(point) => {
                    self.gesture = Some(if usize::from(point.x) < NAV_WIDTH {
                        Gesture::Rail(view_at(point))
                    } else {
                        on_content(content_pointer(point, PointerPhase::Pressed));
                        Gesture::Content
                    });
                }
                TouchEdge::Released(point) => {
                    match self.gesture {
                        Some(Gesture::Rail(Some(view))) if view_at(point) == Some(view) => {
                            selected = Some(view);
                        }
                        Some(Gesture::Content) => {
                            on_content(content_pointer(point, PointerPhase::Released));
                        }
                        _ => {}
                    }
                    self.gesture = None;
                }
            }
        }

        self.forward_movement(&mut on_content);
        if self.gesture.is_none() {
            // Discard movement that belongs to no gesture.
            let _ = self.touch.take_latest_point();
        }
        selected
    }

    fn forward_movement(&mut self, on_content: &mut impl FnMut(Pointer)) {
        let Some(point) = self.touch.take_latest_point() else {
            return;
        };
        match &mut self.gesture {
            Some(Gesture::Rail(candidate)) => {
                if view_at(point) != *candidate {
                    *candidate = None;
                }
            }
            Some(Gesture::Content) => on_content(content_pointer(point, PointerPhase::Moved)),
            None => {}
        }
    }
}

fn content_pointer(point: TouchPoint, phase: PointerPhase) -> Pointer {
    Pointer {
        x: i32::from(point.x) - NAV_WIDTH as i32,
        y: i32::from(point.y),
        phase,
    }
}

fn view_at(point: TouchPoint) -> Option<ViewId> {
    if usize::from(point.x) >= NAV_WIDTH {
        return None;
    }
    let index = (usize::from(point.y) / BUTTON_HEIGHT).min(ViewId::ALL.len() - 1);
    Some(ViewId::ALL[index])
}

/// Draw the rail with `active` highlighted.
pub(crate) fn render(surface: &mut Surface<'_>, active: ViewId) {
    debug_assert_eq!(surface.width(), NAV_WIDTH);
    surface.render_scanlines(|y, pixels| {
        let index = (y / BUTTON_HEIGHT).min(ViewId::ALL.len() - 1);
        let view = ViewId::ALL[index];
        let (background, foreground) = if view == active {
            (pixel::LIGHT_BLUE, pixel::WHITE)
        } else {
            (pixel::DARK_BLUE, pixel::LIGHT_GRAY)
        };

        pixels.fill(background);
        pixels[NAV_WIDTH - 1] = pixel::LIGHT_GRAY;

        let row_in_button = y - index * BUTTON_HEIGHT;
        if let Some(row) = row_in_button.checked_sub(ICON_Y)
            && row < ICON_SIZE
        {
            let bits = view.icon()[row];
            for (column, pixel) in pixels[ICON_X..ICON_X + ICON_SIZE].iter_mut().enumerate() {
                if bits & (1 << (ICON_SIZE - 1 - column)) != 0 {
                    *pixel = foreground;
                }
            }
        }
    });
}
