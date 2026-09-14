//! Where the navigation rail and the content area sit on the panel.

use embedded_graphics::{
    prelude::{Point, Size},
    primitives::Rectangle,
};
use hack_and_hike::capabilities::display;

/// Width of the navigation rail on the left edge.
pub(crate) const NAV_WIDTH: u32 = 44;
/// Size of every screen: the panel minus the rail. The KDL layout files
/// repeat it, and each screen checks at compile time that they agree.
pub(crate) const CONTENT_SIZE: Size =
    Size::new(display::SIZE.width - NAV_WIDTH, display::SIZE.height);
/// The rail, in panel coordinates.
pub(crate) const NAV_AREA: Rectangle =
    Rectangle::new(Point::zero(), Size::new(NAV_WIDTH, display::SIZE.height));
/// The content area, in panel coordinates.
pub(crate) const CONTENT_AREA: Rectangle =
    Rectangle::new(Point::new(NAV_WIDTH as i32, 0), CONTENT_SIZE);
