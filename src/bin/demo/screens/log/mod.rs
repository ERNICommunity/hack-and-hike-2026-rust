//! The device log: the newest lines of everything written through `log`.

use embassy_time::{Duration, Instant};
use embedded_graphics::prelude::Point;
use embedded_gui::Rect;
use hack_and_hike::{
    capabilities::display::Surface,
    support::logging::{HistoryBuffer, LogHistory},
    ui::{
        common,
        gui::{self, GuiSurface},
        theme,
    },
};

use crate::{layout, screens::Screen};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/log/log.kdl");
}

const NODES: usize = 16;
const REFRESH_PERIOD: Duration = Duration::from_millis(100);

pub(crate) struct LogScreen {
    history: LogHistory,
    buffer: HistoryBuffer,
    gui: &'static mut gui::Context<NODES>,
    body: Rect,
    last_refresh: Instant,
    dirty: bool,
}

impl LogScreen {
    pub(crate) fn new(history: LogHistory) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_WIDTH, layout::CONTENT_HEIGHT);
        let app = generated::LogApp::build(gui).expect("log.kdl fits the GUI capacities");
        Self {
            history,
            buffer: HistoryBuffer::new(),
            body: gui::slot(gui, app.widgets.body),
            gui,
            last_refresh: Instant::now(),
            dirty: true,
        }
    }
}

impl Screen for LogScreen {
    fn enter(&mut self) {
        self.dirty = true;
    }

    fn update(&mut self, now: Instant) {
        if now - self.last_refresh < REFRESH_PERIOD {
            return;
        }
        self.last_refresh = now;
        if self.buffer.refresh(&mut self.history) {
            self.dirty = true;
        }
    }

    fn present(&mut self, gui: &mut GuiSurface, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let body = self.body;
        let text = self.buffer.text().unwrap_or_default();
        gui.present(surface, self.gui, |frame| {
            let visible = (body.h as i32 / common::DENSE_LINE_HEIGHT).max(1) as usize;
            let skip = text.lines().count().saturating_sub(visible);
            for (index, line) in text.lines().skip(skip).enumerate() {
                let y = body.y + index as i32 * common::DENSE_LINE_HEIGHT;
                common::text(
                    frame,
                    line,
                    Point::new(body.x, y),
                    common::DENSE_FONT,
                    theme::CHARCOAL,
                );
            }
        });
    }
}
