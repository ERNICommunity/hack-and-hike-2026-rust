//! CPU0 presentation adapter.
//!
//! This module owns Slint/window/touch translation only. Application state and
//! data refresh policy live in `models.rs`.

use alloc::rc::Rc;

use embassy_time::Instant;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};

use crate::{models::{AppModel, ViewId}, screen::Screen, touch, waveform::WaveformFrame};

const SCREEN_WIDTH: u32 = 320;
const SCREEN_HEIGHT: u32 = 240;

slint::include_modules!();

struct McuPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for McuPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(Instant::now().as_millis())
    }
}

#[derive(Clone, Copy)]
struct TouchState {
    pressed: bool,
    last_point: touch::TouchPoint,
}

impl TouchState {
    const fn new() -> Self {
        Self {
            pressed: false,
            last_point: touch::TouchPoint { x: 0, y: 0 },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NavigationChange {
    pub from: ViewId,
    pub to: ViewId,
}

fn logical_position(point: touch::TouchPoint) -> slint::LogicalPosition {
    slint::LogicalPosition {
        x: point.x as f32,
        y: point.y as f32,
    }
}

fn dispatch_move_if_changed(
    window: &MinimalSoftwareWindow,
    state: &mut TouchState,
    point: touch::TouchPoint,
) {
    if point == state.last_point {
        return;
    }

    state.last_point = point;
    window.dispatch_event(WindowEvent::PointerMoved {
        position: logical_position(point),
    });
}

fn dispatch_touch_input(window: &MinimalSoftwareWindow, state: &mut TouchState) {
    while let Some(edge) = touch::try_take_edge() {
        match edge {
            touch::TouchEdge::Pressed(point) => {
                state.pressed = true;
                state.last_point = point;
                window.dispatch_event(WindowEvent::PointerPressed {
                    position: logical_position(point),
                    button: PointerEventButton::Left,
                });
            }
            touch::TouchEdge::Released(point) => {
                if state.pressed {
                    dispatch_move_if_changed(window, state, point);
                }

                window.dispatch_event(WindowEvent::PointerReleased {
                    position: logical_position(point),
                    button: PointerEventButton::Left,
                });
                state.pressed = false;
                state.last_point = point;
            }
        }
    }

    if state.pressed {
        if let Some(point) = touch::take_latest_point() {
            dispatch_move_if_changed(window, state, point);
        }
    }
}

pub struct Ui {
    window: Rc<MinimalSoftwareWindow>,
    app: AppWindow,
    model: Rc<AppModel>,
    touch: TouchState,
    presented_view: ViewId,
}

impl Ui {
    pub fn new(model: Rc<AppModel>) -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(slint::PhysicalSize::new(SCREEN_WIDTH, SCREEN_HEIGHT));

        slint::platform::set_platform(alloc::boxed::Box::new(McuPlatform {
            window: window.clone(),
        }))
        .expect("Failed to initialize Slint platform");

        let app = AppWindow::new().expect("Failed to construct Slint AppWindow");
        app.set_log_lines(model.log_model().into());
        app.set_active_view(model.active_view().as_i32());

        let navigation_model = model.clone();
        app.on_navigate(move |view| navigation_model.request_view(view));

        app.show().expect("Failed to show Slint AppWindow");

        let presented_view = model.active_view();
        Self {
            window,
            app,
            model,
            touch: TouchState::new(),
            presented_view,
        }
    }

    /// Force each permanent page and virtualized delegate set through layout
    /// once. After this returns, interactive navigation should not construct
    /// page/model trees.
    pub fn prewarm_navigation(&mut self, screen: &mut Screen) {
        let initial = self.presented_view;

        for view in ViewId::ALL {
            self.app.set_active_view(view.as_i32());
            slint::platform::update_timers_and_animations();
            screen.render_slint_window(self.window.as_ref());
        }

        self.app.set_active_view(initial.as_i32());
        slint::platform::update_timers_and_animations();
        screen.render_slint_window(self.window.as_ref());
        self.presented_view = initial;
    }

    /// Consume input and update application models, but do not mutate the
    /// Slint page selection yet. This lets the heap monitor take its baseline
    /// before the navigation property setter and renderer run.
    pub fn prepare_frame(&mut self, now: Instant) -> Option<NavigationChange> {
        dispatch_touch_input(&self.window, &mut self.touch);
        self.model.update(now);

        let requested = self.model.active_view();
        let navigation = (requested != self.presented_view).then_some(NavigationChange {
            from: self.presented_view,
            to: requested,
        });

        slint::platform::update_timers_and_animations();
        navigation
    }

    pub fn apply_navigation(&mut self, change: NavigationChange) {
        self.app.set_active_view(change.to.as_i32());
        self.presented_view = change.to;
    }

    pub fn note_slint_redraw(&self, redrawn: bool) {
        self.model.note_slint_redraw(redrawn);
    }

    pub fn take_waveform_frame(&self) -> Option<WaveformFrame> {
        self.model.take_waveform_frame()
    }

    pub fn window(&self) -> &MinimalSoftwareWindow {
        self.window.as_ref()
    }
}
