//! Stock-application navigation state, touch routing, and rail rendering.
//!
//! Navigation is application policy rather than shared UI infrastructure. It
//! owns the active destination and physical touch reader, routing one gesture to
//! either the navigation rail or the active content screen.

use hack_and_hike::capabilities::{
    display::Surface,
    touch::{Touch, TouchEdge, TouchPoint},
};

use super::design;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewId {
    Network,
    Imu,
    Microphone,
    Speaker,
    Camera,
    Settings,
    Log,
}

impl ViewId {
    pub(super) const fn initial() -> Self {
        Self::Log
    }
}

const ICON_X: usize = (design::UI.navigation.width - design::NAV_ICON_SIZE) / 2;
const ICON_Y_IN_BUTTON: usize = (design::UI.navigation.button_height - design::NAV_ICON_SIZE) / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerPhase {
    Pressed,
    Moved,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentPointer {
    pub x: i32,
    pub y: i32,
    pub phase: PointerPhase,
}

#[derive(Clone, Copy)]
enum GestureTarget {
    Navigation(Option<ViewId>),
    Content,
}

pub(super) struct NavigationInput {
    touch: Touch,
    target: Option<GestureTarget>,
}

impl NavigationInput {
    pub(super) const fn new(touch: Touch) -> Self {
        Self {
            touch,
            target: None,
        }
    }

    pub(super) fn poll(&mut self, mut on_content: impl FnMut(ContentPointer)) -> Option<ViewId> {
        let mut selected = None;

        while let Some(edge) = self.touch.next_edge() {
            self.flush_latest_movement(&mut on_content);

            match edge {
                TouchEdge::Pressed(point) => {
                    self.target = if usize::from(point.x) < design::UI.navigation.width {
                        Some(GestureTarget::Navigation(view_at(point)))
                    } else {
                        on_content(content_pointer(point, PointerPhase::Pressed));
                        Some(GestureTarget::Content)
                    };
                }
                TouchEdge::Released(point) => {
                    match self.target {
                        Some(GestureTarget::Navigation(candidate))
                            if candidate.is_some() && candidate == view_at(point) =>
                        {
                            selected = candidate;
                        }
                        Some(GestureTarget::Content) => {
                            on_content(content_pointer(point, PointerPhase::Released));
                        }
                        _ => {}
                    }
                    self.target = None;
                }
            }
        }

        self.flush_latest_movement(&mut on_content);
        if self.target.is_none() {
            let _ = self.touch.take_latest_point();
        }

        selected
    }

    fn flush_latest_movement(&mut self, on_content: &mut impl FnMut(ContentPointer)) {
        let Some(point) = self.touch.take_latest_point() else {
            return;
        };

        match &mut self.target {
            Some(GestureTarget::Navigation(candidate)) => {
                if view_at(point) != *candidate {
                    *candidate = None;
                }
            }
            Some(GestureTarget::Content) => {
                on_content(content_pointer(point, PointerPhase::Moved));
            }
            None => {}
        }
    }
}

fn content_pointer(point: TouchPoint, phase: PointerPhase) -> ContentPointer {
    ContentPointer {
        x: i32::from(point.x) - design::UI.navigation.width as i32,
        y: i32::from(point.y),
        phase,
    }
}

fn view_at(point: TouchPoint) -> Option<ViewId> {
    let nav = design::UI.navigation;
    if usize::from(point.x) >= nav.width {
        return None;
    }

    let index = (usize::from(point.y) / nav.button_height).min(nav.items.len() - 1);
    nav.items.get(index).map(|item| item.view)
}

pub(super) fn render(surface: &mut Surface<'_>, active: ViewId) {
    let nav = design::UI.navigation;
    debug_assert_eq!(surface.width(), nav.width);
    surface.render_scanlines(|screen_y, pixels| {
        let button_index = (screen_y / nav.button_height).min(nav.items.len() - 1);
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

        let button_start = button_index * nav.button_height;
        let local_y = screen_y - button_start;
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
