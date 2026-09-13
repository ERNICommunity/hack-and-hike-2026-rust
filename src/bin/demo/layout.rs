//! Where the navigation rail and the content area sit on the panel.

use hack_and_hike::capabilities::display::{self, Region};

pub(crate) const NAV_WIDTH: usize = 44;
pub(crate) const CONTENT_WIDTH: usize = display::WIDTH - NAV_WIDTH;
pub(crate) const CONTENT_HEIGHT: usize = display::HEIGHT;
pub(crate) const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub(crate) const CONTENT_REGION: Region = Region::new(NAV_WIDTH, 0, CONTENT_WIDTH, CONTENT_HEIGHT);
