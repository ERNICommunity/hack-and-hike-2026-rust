//! The device log: the newest lines of everything written through `log`.
//!
//! Checks ten times a second whether anything was logged, copies the lines
//! that fit the screen and redraws. Useful to see what the firmware does
//! without a serial monitor.

use embassy_time::{Duration, Instant};
use embedded_graphics::{prelude::Point, primitives::Rectangle};
use hack_and_hike::{
    capabilities::display::Surface,
    support::{
        logging::{Line, LogHistory},
        memory::storage,
    },
    ui::{Canvas, common, gui, theme},
};

use crate::{layout, screens::Screen};

// The layout file becomes Rust at compile time: a `...App` struct with a
// `build` function and one `WidgetId` per named node.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/log/log.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// widgets added in code.
const NODES: usize = 16;
/// How often the history is checked for new lines.
const REFRESH_PERIOD: Duration = Duration::from_millis(100);
const _: () = assert!(generated::LogApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::LogApp::HEIGHT == layout::CONTENT_SIZE.height);

/// The log screen and its copy of the newest lines.
pub(crate) struct LogScreen {
    history: LogHistory,
    /// As many lines as fit the body, filled from the history.
    lines: &'static mut [Line],
    /// How many of `lines` are filled.
    shown: usize,
    /// The history revision `lines` was copied at.
    revision: Option<u32>,
    gui: &'static mut gui::Context<NODES>,
    /// Where the lines are drawn.
    body: Rectangle,
    last_refresh: Instant,
    /// Whether the screen needs a redraw.
    dirty: bool,
}

impl LogScreen {
    /// Build the layout and allocate one line buffer per visible row.
    pub(crate) fn new(history: LogHistory) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
        let app = generated::LogApp::build(gui).expect("log.kdl fits the GUI capacities");
        let body = gui::slot(gui, app.widgets.body);
        let visible = (body.size.height as usize / common::DENSE_LINE_HEIGHT as usize).max(1);
        Self {
            history,
            lines: storage::leaked_slice(visible, Line::new()),
            shown: 0,
            revision: None,
            gui,
            body,
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

        let revision = self.history.revision();
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.shown = self.history.newest(self.lines);
            self.dirty = true;
        }
    }

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        canvas.clear(theme::WHITE);
        gui::render(self.gui, canvas);
        for (index, line) in self.lines[..self.shown].iter().enumerate() {
            let origin =
                self.body.top_left + Point::new(0, index as i32 * common::DENSE_LINE_HEIGHT);
            common::text(canvas, line, origin, common::DENSE_FONT, theme::CHARCOAL);
        }
        canvas.show(surface);
    }
}
