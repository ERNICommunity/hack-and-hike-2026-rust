//! The navigation rail on the left and the routing of touches.
//!
//! A touch that starts on the rail selects a screen when it is released on
//! the same button. A touch that starts in the content area is forwarded to
//! the visible screen in content coordinates.

use embedded_graphics::prelude::Point;
use hack_and_hike::{
    capabilities::{
        display::{self, Surface},
        touch::{Touch, TouchEvent},
    },
    ui::theme,
};

use crate::layout::NAV_WIDTH;

/// Names one screen of the demo, and its button on the rail.
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

    /// The rail button's picture.
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

/// Icons are 16 x 16 pixels.
const ICON_SIZE: usize = 16;
/// A 1-bit icon: one `u16` per row, most significant bit on the left, a set
/// bit is a lit pixel. Write `0x0180` as binary to see the shape.
type Icon = [u16; ICON_SIZE];

/// The rail is split evenly between the buttons.
const BUTTON_HEIGHT: usize = display::HEIGHT / ViewId::ALL.len();
/// Where the icon starts inside its button, to centre it.
const ICON_X: usize = (NAV_WIDTH as usize - ICON_SIZE) / 2;
const ICON_Y: usize = (BUTTON_HEIGHT - ICON_SIZE) / 2;
const _: () = assert!(BUTTON_HEIGHT > ICON_SIZE);

// The icons, top row first.
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

/// Where the current touch started.
#[derive(Clone, Copy)]
enum Gesture {
    /// On a rail button; `None` once the finger left that button, which
    /// cancels the selection.
    Rail(Option<ViewId>),
    /// In the content area; the visible screen gets every event until the
    /// finger lifts.
    Content,
}

/// Owns the touch handle and turns raw touches into screen selections and
/// content touches.
pub(crate) struct Navigation {
    touch: Touch,
    /// The touch in progress; `None` while no finger is down.
    gesture: Option<Gesture>,
}

impl Navigation {
    /// Route the touches of `touch`.
    pub(crate) const fn new(touch: Touch) -> Self {
        Self {
            touch,
            gesture: None,
        }
    }

    /// Process all pending touches. Content touches go to `on_content`; the
    /// return value is a screen the user selected on the rail, if any.
    pub(crate) fn poll(&mut self, mut on_content: impl FnMut(TouchEvent)) -> Option<ViewId> {
        let mut selected = None;

        while let Some(event) = self.touch.next_event() {
            match (event, self.gesture) {
                (TouchEvent::Pressed(point), _) => {
                    self.gesture = Some(if point.x < NAV_WIDTH as i32 {
                        Gesture::Rail(view_at(point))
                    } else {
                        on_content(in_content_coordinates(event));
                        Gesture::Content
                    });
                }
                (TouchEvent::Moved(point), Some(Gesture::Rail(candidate))) => {
                    if view_at(point) != candidate {
                        self.gesture = Some(Gesture::Rail(None));
                    }
                }
                (TouchEvent::Released(point), Some(Gesture::Rail(candidate))) => {
                    if candidate.is_some() && view_at(point) == candidate {
                        selected = candidate;
                    }
                    self.gesture = None;
                }
                (TouchEvent::Moved(_), Some(Gesture::Content)) => {
                    on_content(in_content_coordinates(event));
                }
                (TouchEvent::Released(_), Some(Gesture::Content)) => {
                    on_content(in_content_coordinates(event));
                    self.gesture = None;
                }
                (_, None) => {}
            }
        }
        selected
    }
}

/// The same event with its point relative to the content area's top-left
/// corner, the coordinates every screen works in.
fn in_content_coordinates(event: TouchEvent) -> TouchEvent {
    let shift = |point: Point| point - Point::new(NAV_WIDTH as i32, 0);
    match event {
        TouchEvent::Pressed(point) => TouchEvent::Pressed(shift(point)),
        TouchEvent::Moved(point) => TouchEvent::Moved(shift(point)),
        TouchEvent::Released(point) => TouchEvent::Released(shift(point)),
    }
}

/// The rail button under `point`, or `None` outside the rail.
fn view_at(point: Point) -> Option<ViewId> {
    if point.x < 0 || point.x >= NAV_WIDTH as i32 || point.y < 0 {
        return None;
    }
    let index = (point.y as usize / BUTTON_HEIGHT).min(ViewId::ALL.len() - 1);
    Some(ViewId::ALL[index])
}

/// Draw the rail with `active` highlighted.
pub(crate) fn render(surface: &mut Surface<'_>, active: ViewId) {
    debug_assert_eq!(surface.width(), NAV_WIDTH as usize);
    surface.render_scanlines(|y, row| {
        let index = (y / BUTTON_HEIGHT).min(ViewId::ALL.len() - 1);
        let view = ViewId::ALL[index];
        let (background, foreground) = if view == active {
            (theme::LIGHT_BLUE, theme::WHITE)
        } else {
            (theme::DARK_BLUE, theme::LIGHT_GRAY)
        };

        row.fill(background);

        let row_in_button = y - index * BUTTON_HEIGHT;
        if let Some(icon_row) = row_in_button.checked_sub(ICON_Y)
            && icon_row < ICON_SIZE
        {
            let bits = view.icon()[icon_row];
            for (column, pixel) in row[ICON_X..ICON_X + ICON_SIZE].iter_mut().enumerate() {
                if bits & (1 << (ICON_SIZE - 1 - column)) != 0 {
                    *pixel = foreground;
                }
            }
        }
    });
}
