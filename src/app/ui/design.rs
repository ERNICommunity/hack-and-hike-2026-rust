//! Compile-time presentation specification for firmware-owned UI chrome.
//!
//! KDL owns all content-view geometry. This module retains only the physical
//! content/navigation partition, typed content-to-screen conversion used by
//! specialized direct renderers, and navigation-rail styling/order.

use crate::{
    display::{self, Region},
    models::ViewId,
    theme,
};

const NAV_WIDTH: usize = 44;
pub(crate) const NAV_ICON_SIZE: usize = 16;
pub(crate) const NAV_ITEM_COUNT: usize = 7;
const NAV_BUTTON_HEIGHT: usize = display::HEIGHT / NAV_ITEM_COUNT;

pub(crate) const CONTENT_WIDTH: usize = display::WIDTH - NAV_WIDTH;
pub(crate) const CONTENT_HEIGHT: usize = display::HEIGHT;
pub(crate) const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub(crate) const CONTENT_REGION: Region =
    Region::new(NAV_WIDTH, 0, CONTENT_WIDTH, CONTENT_HEIGHT);

pub(crate) type NavIcon = [u16; NAV_ICON_SIZE];
pub(crate) const CAMERA_ICON: NavIcon = [
    0x0000, 0x0000, 0x0F00, 0x1980, 0x7FFE, 0x4002, 0x43C2, 0x4662, 0x4C32, 0x4C32,
    0x4662, 0x43C2, 0x4002, 0x7FFE, 0x0000, 0x0000,
];

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
///
/// KDL determines these coordinates; this type only performs the explicit
/// conversion required by direct display paths such as the microphone waveform.
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

    pub(crate) const fn screen_region(self) -> Region {
        Region::new(NAV_WIDTH + self.x, self.y, self.width, self.height)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NavigationItemSpec {
    pub view: ViewId,
    pub icon: NavIcon,
}

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

#[derive(Clone, Copy, Debug)]
pub(crate) struct UiDesign {
    pub navigation: NavigationSpec,
}

const WHITE: UiColor = UiColor::from_rgb565(theme::WHITE_RGB565);
const DARK_BLUE: UiColor = UiColor::from_rgb565(theme::DARK_BLUE_RGB565);
const LIGHT_BLUE: UiColor = UiColor::from_rgb565(theme::LIGHT_BLUE_RGB565);
const DARK_GRAY: UiColor = UiColor::from_rgb565(theme::DARK_GRAY_RGB565);

pub(crate) const UI: UiDesign = UiDesign {
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
                view: ViewId::Camera,
                icon: CAMERA_ICON,
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
};

const _: () = assert!(NAV_WIDTH < display::WIDTH);
const _: () = assert!(NAV_BUTTON_HEIGHT > NAV_ICON_SIZE);
const _: () = assert!(NAV_BUTTON_HEIGHT * NAV_ITEM_COUNT <= display::HEIGHT);
const _: () = assert!(UI.navigation.width == NAV_WIDTH);
