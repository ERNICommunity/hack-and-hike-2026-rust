//! Compile-time presentation specification for firmware-owned UI chrome.
//!
//! KDL owns the geometry of embedded-gui screens. This module intentionally
//! retains only layout and styling owned by the firmware itself: navigation,
//! legacy text/placeholder views, and the bespoke IMU renderer.
//!
//! `ContentRect` is deliberately distinct from `display::Region`: content-space
//! coordinates are relative to the 276x240 content area, while `display::Region`
//! uses physical panel coordinates. Converting between them is explicit.

use crate::{
    display::{self, Region},
    models::ViewId,
    theme,
};

const NAV_WIDTH: usize = 44;
const NAV_BUTTON_HEIGHT: usize = 40;

pub(crate) const CONTENT_WIDTH: usize = display::WIDTH - NAV_WIDTH;
pub(crate) const CONTENT_HEIGHT: usize = display::HEIGHT;
pub(crate) const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub(crate) const CONTENT_REGION: Region =
    Region::new(NAV_WIDTH, 0, CONTENT_WIDTH, CONTENT_HEIGHT);

pub(crate) const NAV_ICON_SIZE: usize = 16;
pub(crate) const NAV_ITEM_COUNT: usize = 6;
pub(crate) type NavIcon = [u16; NAV_ICON_SIZE];

/// Semantic UI color stored in the display's native RGB565 representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UiColor(u16);

impl UiColor {
    pub(crate) const fn from_rgb565(raw: u16) -> Self {
        Self(raw)
    }

    pub(crate) const fn raw(self) -> u16 {
        self.0
    }
}

/// Valid non-empty rectangle in content coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl ContentRect {
    pub(crate) const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        assert!(width > 0 && height > 0);
        assert!(x <= CONTENT_WIDTH && width <= CONTENT_WIDTH - x);
        assert!(y <= CONTENT_HEIGHT && height <= CONTENT_HEIGHT - y);
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) const fn x(self) -> usize {
        self.x
    }

    pub(crate) const fn y(self) -> usize {
        self.y
    }

    pub(crate) const fn width(self) -> usize {
        self.width
    }

    pub(crate) const fn height(self) -> usize {
        self.height
    }

    pub(crate) const fn screen_region(self) -> Region {
        Region::new(NAV_WIDTH + self.x, self.y, self.width, self.height)
    }
}

/// One navigation destination and its fixed bitmap icon.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NavigationItemSpec {
    pub view: ViewId,
    pub icon: NavIcon,
}

/// Fixed navigation-rail geometry, ordering, icons, and semantic colors.
///
/// Hit testing and rendering consume the same `items` array, so page order and
/// icon order cannot drift apart.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NavigationSpec {
    pub width: usize,
    pub button_height: usize,
    pub items: [NavigationItemSpec; NAV_ITEM_COUNT],
    pub selected_background: UiColor,
    pub normal_background: UiColor,
    pub selected_icon: UiColor,
    pub normal_icon: UiColor,
    pub divider: UiColor,
}

/// Shared monospaced page layout used by Network and Log.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextPageSpec {
    pub x: usize,
    pub top: usize,
    pub line_height: usize,
    pub visible_lines: usize,
    pub foreground: UiColor,
    pub background: UiColor,
}

/// Centered placeholder page styling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlaceholderSpec {
    pub title_y: usize,
    pub subtitle_y: usize,
    pub title: UiColor,
    pub subtitle: UiColor,
    pub background: UiColor,
}

/// Header column positions for the three attitude values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImuHeaderColumns {
    pub roll_x: usize,
    pub pitch_x: usize,
    pub yaw_x: usize,
}

/// Major IMU view regions and semantic colors.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImuSpec {
    pub header: ContentRect,
    pub attitude: ContentRect,
    pub compass: ContentRect,
    pub header_columns: ImuHeaderColumns,
    pub background: UiColor,
    pub primary: UiColor,
    pub horizon_sky: UiColor,
    pub secondary: UiColor,
    pub border: UiColor,
    pub on_primary: UiColor,
}

/// Complete firmware-owned design consumed outside KDL screens.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UiDesign {
    pub content_background: UiColor,
    pub navigation: NavigationSpec,
    pub text: TextPageSpec,
    pub placeholder: PlaceholderSpec,
    pub imu: ImuSpec,
}

const WHITE: UiColor = UiColor::from_rgb565(theme::WHITE_RGB565);
const BLACK: UiColor = UiColor::from_rgb565(theme::BLACK_RGB565);
const DARK_BLUE: UiColor = UiColor::from_rgb565(theme::DARK_BLUE_RGB565);
const LIGHT_BLUE: UiColor = UiColor::from_rgb565(theme::LIGHT_BLUE_RGB565);
const DARK_GRAY: UiColor = UiColor::from_rgb565(theme::DARK_GRAY_RGB565);
const LIGHT_GRAY: UiColor = UiColor::from_rgb565(theme::LIGHT_GRAY_RGB565);

pub(crate) const UI: UiDesign = UiDesign {
    content_background: WHITE,
    navigation: NavigationSpec {
        width: NAV_WIDTH,
        button_height: NAV_BUTTON_HEIGHT,
        items: [
            NavigationItemSpec {
                view: ViewId::Network,
                icon: [
                    0x0000, 0x0000, 0x0180, 0x03C0, 0x0660, 0x0C30, 0x1818, 0x0180,
                    0x0180, 0x1818, 0x0C30, 0x0660, 0x03C0, 0x0180, 0x0000, 0x0000,
                ],
            },
            NavigationItemSpec {
                view: ViewId::Imu,
                icon: [
                    0x0180, 0x0180, 0x0180, 0x0180, 0x0180, 0x7FFE, 0x0180, 0x0180,
                    0x0180, 0x0180, 0x07E0, 0x0DB0, 0x198C, 0x0180, 0x0180, 0x0000,
                ],
            },
            NavigationItemSpec {
                view: ViewId::Microphone,
                icon: [
                    0x03C0, 0x0660, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30,
                    0x0660, 0x03C0, 0x0180, 0x1FF8, 0x0180, 0x0180, 0x07E0, 0x0000,
                ],
            },
            NavigationItemSpec {
                view: ViewId::Speaker,
                icon: [
                    0x0000, 0x0300, 0x0700, 0x0F18, 0x7F0C, 0x7F06, 0x7F06, 0x7F06,
                    0x7F06, 0x7F06, 0x7F0C, 0x0F18, 0x0700, 0x0300, 0x0000, 0x0000,
                ],
            },
            NavigationItemSpec {
                view: ViewId::Settings,
                icon: [
                    0x0000, 0x0180, 0x0DB0, 0x1FF8, 0x319C, 0x6186, 0x6786, 0x6606,
                    0x6606, 0x6786, 0x6186, 0x319C, 0x1FF8, 0x0DB0, 0x0180, 0x0000,
                ],
            },
            NavigationItemSpec {
                view: ViewId::Log,
                icon: [
                    0x0000, 0x0000, 0x3FFC, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004,
                    0x2FF4, 0x2004, 0x2FF4, 0x2004, 0x3FFC, 0x0000, 0x0000, 0x0000,
                ],
            },
        ],
        selected_background: LIGHT_BLUE,
        normal_background: DARK_BLUE,
        selected_icon: WHITE,
        normal_icon: DARK_GRAY,
        divider: DARK_BLUE,
    },
    text: TextPageSpec {
        x: 4,
        top: 4,
        line_height: 10,
        visible_lines: 23,
        foreground: BLACK,
        background: WHITE,
    },
    placeholder: PlaceholderSpec {
        title_y: 92,
        subtitle_y: 114,
        title: DARK_BLUE,
        subtitle: DARK_GRAY,
        background: WHITE,
    },
    imu: ImuSpec {
        header: ContentRect::new(6, 6, CONTENT_WIDTH - 12, 44),
        attitude: ContentRect::new(6, 56, CONTENT_WIDTH - 12, 140),
        compass: ContentRect::new(6, 202, CONTENT_WIDTH - 12, 32),
        header_columns: ImuHeaderColumns {
            roll_x: 83,
            pitch_x: 143,
            yaw_x: 207,
        },
        background: WHITE,
        primary: DARK_BLUE,
        horizon_sky: LIGHT_BLUE,
        secondary: DARK_GRAY,
        border: LIGHT_GRAY,
        on_primary: WHITE,
    },
};

const _: () = assert!(NAV_WIDTH < display::WIDTH);
const _: () = assert!(NAV_BUTTON_HEIGHT * NAV_ITEM_COUNT == display::HEIGHT);
const _: () = assert!(UI.navigation.width == NAV_WIDTH);
const _: () = assert!(UI.text.line_height > 0);
const _: () = assert!(
    UI.text.top + UI.text.visible_lines * UI.text.line_height <= CONTENT_HEIGHT
);
const _: () = assert!(UI.placeholder.title_y < CONTENT_HEIGHT);
const _: () = assert!(UI.placeholder.subtitle_y < CONTENT_HEIGHT);
const _: () = assert!(UI.imu.header_columns.roll_x < CONTENT_WIDTH);
const _: () = assert!(UI.imu.header_columns.pitch_x < CONTENT_WIDTH);
const _: () = assert!(UI.imu.header_columns.yaw_x < CONTENT_WIDTH);
