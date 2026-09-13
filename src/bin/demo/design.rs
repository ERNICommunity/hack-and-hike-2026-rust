//! Compile-time presentation specification for the stock application's chrome.
//!
//! KDL owns all content-screen geometry. This module retains only the physical
//! content/navigation partition and stock navigation-rail styling/order.

use hack_and_hike::{
    capabilities::display::{self, Region},
    ui::theme,
};

use super::navigation::ViewId;

const NAV_WIDTH: usize = 44;
pub(super) const NAV_ICON_SIZE: usize = 16;

pub(super) const CONTENT_WIDTH: usize = display::WIDTH - NAV_WIDTH;
pub(super) const CONTENT_HEIGHT: usize = display::HEIGHT;
pub(super) const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub(super) const CONTENT_REGION: Region = Region::new(NAV_WIDTH, 0, CONTENT_WIDTH, CONTENT_HEIGHT);

pub(super) type NavIcon = [u16; NAV_ICON_SIZE];
pub(super) const CAMERA_ICON: NavIcon = [
    0x0000, 0x0000, 0x0F00, 0x1980, 0x7FFE, 0x4002, 0x43C2, 0x4662, 0x4C32, 0x4C32, 0x4662, 0x43C2,
    0x4002, 0x7FFE, 0x0000, 0x0000,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UiColor(u16);

impl UiColor {
    pub(super) const fn from_rgb565(raw: u16) -> Self {
        Self(raw)
    }

    pub(super) const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NavigationItemSpec {
    pub view: ViewId,
    pub icon: NavIcon,
}

const NAV_ITEMS: &[NavigationItemSpec] = &[
    NavigationItemSpec {
        view: ViewId::Network,
        icon: [
            0x0000, 0x0000, 0x0180, 0x03C0, 0x0660, 0x0C30, 0x1818, 0x0180, 0x0180, 0x1818, 0x0C30,
            0x0660, 0x03C0, 0x0180, 0x0000, 0x0000,
        ],
    },
    NavigationItemSpec {
        view: ViewId::Imu,
        icon: [
            0x0180, 0x0180, 0x0180, 0x0180, 0x0180, 0x7FFE, 0x0180, 0x0180, 0x0180, 0x0180, 0x07E0,
            0x0DB0, 0x198C, 0x0180, 0x0180, 0x0000,
        ],
    },
    NavigationItemSpec {
        view: ViewId::Microphone,
        icon: [
            0x03C0, 0x0660, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0C30, 0x0660, 0x03C0, 0x0180,
            0x1FF8, 0x0180, 0x0180, 0x07E0, 0x0000,
        ],
    },
    NavigationItemSpec {
        view: ViewId::Speaker,
        icon: [
            0x0000, 0x0300, 0x0700, 0x0F18, 0x7F0C, 0x7F06, 0x7F06, 0x7F06, 0x7F06, 0x7F06, 0x7F0C,
            0x0F18, 0x0700, 0x0300, 0x0000, 0x0000,
        ],
    },
    NavigationItemSpec {
        view: ViewId::Camera,
        icon: CAMERA_ICON,
    },
    NavigationItemSpec {
        view: ViewId::Settings,
        icon: [
            0x0000, 0x0180, 0x0DB0, 0x1FF8, 0x319C, 0x6186, 0x6786, 0x6606, 0x6606, 0x6786, 0x6186,
            0x319C, 0x1FF8, 0x0DB0, 0x0180, 0x0000,
        ],
    },
    NavigationItemSpec {
        view: ViewId::Log,
        icon: [
            0x0000, 0x0000, 0x3FFC, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004, 0x2FF4, 0x2004, 0x2FF4,
            0x2004, 0x3FFC, 0x0000, 0x0000, 0x0000,
        ],
    },
];

const NAV_ITEM_COUNT: usize = NAV_ITEMS.len();
const NAV_BUTTON_HEIGHT: usize = display::HEIGHT / NAV_ITEM_COUNT;

#[derive(Clone, Copy, Debug)]
pub(super) struct NavigationSpec {
    pub width: usize,
    pub button_height: usize,
    pub items: &'static [NavigationItemSpec],
    pub selected_background: UiColor,
    pub normal_background: UiColor,
    pub selected_icon: UiColor,
    pub normal_icon: UiColor,
    pub divider: UiColor,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UiDesign {
    pub navigation: NavigationSpec,
}

const WHITE: UiColor = UiColor::from_rgb565(theme::WHITE_RGB565);
const DARK_BLUE: UiColor = UiColor::from_rgb565(theme::DARK_BLUE_RGB565);
const LIGHT_BLUE: UiColor = UiColor::from_rgb565(theme::LIGHT_BLUE_RGB565);
const DARK_GRAY: UiColor = UiColor::from_rgb565(theme::DARK_GRAY_RGB565);

pub(super) const UI: UiDesign = UiDesign {
    navigation: NavigationSpec {
        width: NAV_WIDTH,
        button_height: NAV_BUTTON_HEIGHT,
        items: NAV_ITEMS,
        selected_background: LIGHT_BLUE,
        normal_background: DARK_BLUE,
        selected_icon: WHITE,
        normal_icon: DARK_GRAY,
        divider: DARK_BLUE,
    },
};

const _: () = assert!(NAV_ITEM_COUNT > 0);
const _: () = assert!(NAV_WIDTH < display::WIDTH);
const _: () = assert!(NAV_BUTTON_HEIGHT > NAV_ICON_SIZE);
const _: () = assert!(NAV_BUTTON_HEIGHT * NAV_ITEM_COUNT <= display::HEIGHT);
const _: () = assert!(UI.navigation.width == NAV_WIDTH);
