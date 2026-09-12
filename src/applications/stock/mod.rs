//! Default Hack & Hike application.
//!
//! This one application owns every capability enabled by `app-stock`, its
//! navigation, all stock screens, and the policy for scheduling/rendering them.
//! Shared UI code only provides drawing primitives; no firmware-global shell
//! knows about these destinations.

mod design;
mod navigation;
mod views;

use ::log::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};

use crate::{
    capabilities::{camera, display::Display},
    firmware::Bootstrap,
    support::memory::HeapMonitor,
    ui::gui::GuiSurface,
};

use navigation::{NavigationInput, ViewId};

const UI_IDLE_DELAY: Duration = Duration::from_millis(5);
// High-rate screens used to spin without ever returning to the CPU0 executor.
// Keep their throughput while guaranteeing a cooperative scheduling point.
const HIGH_RATE_YIELD_DELAY: Duration = Duration::from_micros(1);

#[derive(Clone, Copy, Debug)]
struct ViewTransition {
    from: ViewId,
    to: ViewId,
}

struct Views {
    network_demo: views::network_demo::Application,
    imu_worldview: views::imu_worldview::Application,
    mic_waveform: views::mic_waveform::Application,
    speaker_synth: views::speaker_synth::Application,
    camera_view: views::camera_view::Application,
    settings: views::settings::Application,
    log_view: views::log_view::Application,
}

struct Ui {
    views: Views,
    navigation: NavigationInput,
    gui_surface: GuiSurface,
    active_view: ViewId,
    presented_view: ViewId,
}

impl Ui {
    fn new(views: Views, touch: crate::capabilities::touch::Touch) -> Self {
        let active_view = ViewId::initial();
        Self {
            views,
            navigation: NavigationInput::new(touch),
            gui_surface: GuiSurface::new(),
            active_view,
            presented_view: active_view,
        }
    }

    fn presented_view(&self) -> ViewId {
        self.presented_view
    }

    fn render_initial(&mut self, display: &mut Display) {
        {
            let mut navigation_surface = display.surface(design::NAV_REGION);
            navigation::render(&mut navigation_surface, self.presented_view);
        }
        self.present_current_view(display);
    }

    fn prepare_frame(&mut self, now: Instant) -> Option<ViewTransition> {
        let active_view = self.active_view;
        let selected = {
            let navigation = &mut self.navigation;
            let views = &mut self.views;
            navigation.poll(|pointer| match active_view {
                ViewId::Settings => views.settings.handle_pointer(pointer),
                ViewId::Speaker => views.speaker_synth.handle_pointer(pointer),
                _ => {}
            })
        };

        if let Some(view) = selected {
            if view != self.active_view {
                self.active_view = view;
                self.mark_active_dirty();
            }
        }

        // These behaviors intentionally continue while another stock screen is
        // visible. The scheduling policy now belongs to this application.
        self.views.speaker_synth.update();
        self.views.network_demo.update_if_due(now);

        match self.active_view {
            ViewId::Network => {}
            ViewId::Imu => self.views.imu_worldview.update_if_due(now),
            ViewId::Microphone => self.views.mic_waveform.update_if_due(now),
            ViewId::Speaker => {}
            ViewId::Camera => {}
            ViewId::Settings => {}
            ViewId::Log => self.views.log_view.update_if_due(now),
        }

        (self.active_view != self.presented_view).then_some(ViewTransition {
            from: self.presented_view,
            to: self.active_view,
        })
    }

    fn mark_active_dirty(&mut self) {
        match self.active_view {
            ViewId::Network => self.views.network_demo.mark_dirty(),
            ViewId::Imu => self.views.imu_worldview.mark_dirty(),
            ViewId::Microphone => self.views.mic_waveform.mark_dirty(),
            ViewId::Speaker => self.views.speaker_synth.mark_dirty(),
            ViewId::Camera => {}
            ViewId::Settings => self.views.settings.mark_dirty(),
            ViewId::Log => self.views.log_view.mark_dirty(),
        }
    }

    fn apply_navigation(&mut self, transition: ViewTransition, display: &mut Display) {
        debug_assert_eq!(transition.from, self.presented_view);
        debug_assert_eq!(transition.to, self.active_view);
        self.presented_view = transition.to;
        {
            let mut navigation_surface = display.surface(design::NAV_REGION);
            navigation::render(&mut navigation_surface, transition.to);
        }
        self.present_current_view(display);
    }

    fn render(&mut self, display: &mut Display) {
        let mut content = display.surface(design::CONTENT_REGION);
        match self.presented_view {
            ViewId::Network => {
                let _ = self
                    .views
                    .network_demo
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            ViewId::Imu => {
                let _ = self
                    .views
                    .imu_worldview
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            ViewId::Microphone => {
                let _ = self.views.mic_waveform.render_if_dirty(&mut content);
            }
            ViewId::Speaker => {
                let _ = self
                    .views
                    .speaker_synth
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            ViewId::Camera => {}
            ViewId::Settings => {
                let _ = self
                    .views
                    .settings
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            ViewId::Log => {
                let _ = self
                    .views
                    .log_view
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
        }
    }

    fn render_camera(&self, display: &mut Display, frame: &mut camera::Frame<'_>) {
        if self.presented_view == ViewId::Camera {
            let mut content = display.surface(design::CONTENT_REGION);
            self.views.camera_view.render(&mut content, frame);
        }
    }

    fn present_current_view(&mut self, display: &mut Display) {
        let mut content = display.surface(design::CONTENT_REGION);
        match self.presented_view {
            ViewId::Network => self
                .views
                .network_demo
                .present_shell(&mut self.gui_surface, &mut content),
            ViewId::Imu => self
                .views
                .imu_worldview
                .present_shell(&mut self.gui_surface, &mut content),
            ViewId::Microphone => self
                .views
                .mic_waveform
                .present_shell(&mut self.gui_surface, &mut content),
            ViewId::Speaker => {
                let presented = self
                    .views
                    .speaker_synth
                    .present_if_dirty(&mut self.gui_surface, &mut content);
                debug_assert!(presented, "speaker state must be dirty when entering Speaker");
            }
            ViewId::Camera => self.views.camera_view.present_shell(&mut content),
            ViewId::Settings => {
                let presented = self
                    .views
                    .settings
                    .present_if_dirty(&mut self.gui_surface, &mut content);
                debug_assert!(presented, "settings state must be dirty when entering Settings");
            }
            ViewId::Log => self
                .views
                .log_view
                .present_current(&mut self.gui_surface, &mut content),
        }
    }
}

pub(crate) async fn run(_spawner: Spawner, bootstrap: Bootstrap) -> ! {
    let Bootstrap {
        mut display,
        touch,
        imu,
        microphone,
        speaker,
        network,
        mut camera,
        camera_ready,
        brightness,
        log,
    } = bootstrap;

    let views = Views {
        network_demo: views::network_demo::Application::new(network),
        imu_worldview: views::imu_worldview::Application::new(imu),
        mic_waveform: views::mic_waveform::Application::new(microphone),
        speaker_synth: views::speaker_synth::Application::new(speaker),
        camera_view: views::camera_view::Application::new(),
        settings: views::settings::Application::new(brightness),
        log_view: views::log_view::Application::new(log),
    };
    let mut ui = Ui::new(views, touch);

    let now = Instant::now();
    let mut heap_monitor = HeapMonitor::new(now);
    heap_monitor.checkpoint("after stock application construction");

    ui.render_initial(&mut display);
    heap_monitor.checkpoint("after initial stock render");

    loop {
        let now = Instant::now();
        let transition = ui.prepare_frame(now);

        if let Some(transition) = transition {
            heap_monitor.begin_activity(transition.to.name());
            if transition.from == ViewId::Camera {
                camera.pause();
            }
            ui.apply_navigation(transition, &mut display);
        }

        ui.render(&mut display);

        let camera_active = camera_ready && ui.presented_view() == ViewId::Camera;
        if camera_active {
            if let Some(mut frame) = camera.begin_frame() {
                ui.render_camera(&mut display, &mut frame);
                frame.finish();
            }
        }

        if let Some(transition) = transition {
            heap_monitor.end_activity();
            info!("View {:?} -> {:?}", transition.from, transition.to);
        }
        heap_monitor.poll(now);

        let imu_active = ui.presented_view() == ViewId::Imu;
        let delay = if camera_active || imu_active {
            HIGH_RATE_YIELD_DELAY
        } else {
            UI_IDLE_DELAY
        };
        Timer::after(delay).await;
    }
}
