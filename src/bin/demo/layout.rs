//! Where the navigation rail and the content area sit on the panel.

use embedded_graphics::{
    prelude::{Point, Size},
    primitives::Rectangle,
};
use hack_and_hike::capabilities::display;

pub(crate) const NAV_WIDTH: u32 = 44;
pub(crate) const CONTENT_SIZE: Size =
    Size::new(display::SIZE.width - NAV_WIDTH, display::SIZE.height);
pub(crate) const NAV_AREA: Rectangle =
    Rectangle::new(Point::zero(), Size::new(NAV_WIDTH, display::SIZE.height));
pub(crate) const CONTENT_AREA: Rectangle =
    Rectangle::new(Point::new(NAV_WIDTH as i32, 0), CONTENT_SIZE);
