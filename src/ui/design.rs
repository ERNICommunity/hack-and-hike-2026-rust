//! Declarative, compile-time presentation specification.
//!
//! This module is the editable design surface for the firmware UI. Geometry and
//! semantic color assignments are plain `const` Rust data consumed by the
//! renderers. Layout edits therefore remain allocation-free and are validated at
//! compile time rather than becoming runtime clipping or indexing assumptions.
//!
//! `ContentRect` is deliberately distinct from `display::Region`: content-space
//! coordinates are relative to the framebuffer, while `display::Region` uses
//! physical panel coordinates. Converting between them is explicit.

use crate::{
    display::{self, Region},
    models::ViewId,
    theme,
    waveform,
};

const NAV_WIDTH: usize = 44;
const NAV_BUTTON_HEIGHT: usize = 48;

pub(crate) const CONTENT_WIDTH: usize = display::WIDTH - NAV_WIDTH;
pub(crate) const CONTENT_HEIGHT: usize = display::HEIGHT;
pub(crate) const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub(crate) const CONTENT_REGION: Region =
    Region::new(NAV_WIDTH, 0, CONTENT_WIDTH, CONTENT_HEIGHT);

pub(crate) const NAV_ICON_SIZE: usize = 16;
pub(crate) const NAV_ITEM_COUNT: usize = 5;
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

/// Valid non-empty rectangle in content-framebuffer coordinates.
///
/// Fields are private and construction checks the fixed content bounds. Every
/// `ContentRect` in `UI` is therefore safe to draw or translate to the panel.
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

    pub(crate) const fn contains(self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
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

/// One microphone channel's label and waveform canvas in content coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaveformPanelSpec {
    pub label: &'static str,
    pub panel: ContentRect,
    pub label_x_offset: usize,
    pub label_y_offset: usize,
    pub canvas: ContentRect,
}

/// Static microphone labels and high-rate waveform styling.
///
/// Left and right are named fields because channel identity is semantic, not an
/// array position contract. The page itself uses the normal white content
/// background; only labels, grid, and traces add ink.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MicrophoneSpec {
    pub left: WaveformPanelSpec,
    pub right: WaveformPanelSpec,
    pub label: UiColor,
    pub canvas_background: UiColor,
    pub grid: UiColor,
    pub trace: UiColor,
}

/// Header column positions for the three attitude values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImuHeaderColumns {
    pub roll_x: usize,
    pub pitch_x: usize,
    pub yaw_x: usize,
}

/// Major IMU view regions and semantic colors.
///
/// Fine attitude-marker geometry remains renderer-local because it describes the
/// graphic itself; page sizing and palette choices live here.
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

/// Complete compile-time design consumed by all CPU0 view renderers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UiDesign {
    pub content_background: UiColor,
    pub navigation: NavigationSpec,
    pub text: TextPageSpec,
    pub placeholder: PlaceholderSpec,
    pub microphone: MicrophoneSpec,
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
                view: ViewId::Sound,
                icon: [
                    0x0000, 0x0300, 0x0700, 0x0F18, 0x7F0C, 0x7F06, 0x7F06, 0x7F06,
                    0x7F06, 0x7F06, 0x7F0C, 0x0F18, 0x0700, 0x0300, 0x0000, 0x0000,
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
    microphone: MicrophoneSpec {
        left: WaveformPanelSpec {
            label: "MIC L",
            panel: ContentRect::new(0, 0, CONTENT_WIDTH, 120),
            label_x_offset: 10,
            label_y_offset: 2,
            canvas: ContentRect::new(10, 14, 256, 104),
        },
        right: WaveformPanelSpec {
            label: "MIC R",
            panel: ContentRect::new(0, 120, CONTENT_WIDTH, 120),
            label_x_offset: 10,
            label_y_offset: 2,
            canvas: ContentRect::new(10, 134, 256, 104),
        },
        label: BLACK,
        canvas_background: WHITE,
        grid: LIGHT_GRAY,
        trace: DARK_BLUE,
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
const _: () = assert!(UI.microphone.left.panel.contains(UI.microphone.left.canvas));
const _: () = assert!(UI.microphone.right.panel.contains(UI.microphone.right.canvas));
const _: () = assert!(UI.microphone.left.label_x_offset < UI.microphone.left.panel.width());
const _: () = assert!(UI.microphone.left.label_y_offset < UI.microphone.left.panel.height());
const _: () = assert!(UI.microphone.right.label_x_offset < UI.microphone.right.panel.width());
const _: () = assert!(UI.microphone.right.label_y_offset < UI.microphone.right.panel.height());
const _: () = assert!(UI.microphone.left.canvas.width() % waveform::POINTS == 0);
const _: () = assert!(UI.microphone.right.canvas.width() % waveform::POINTS == 0);
const _: () = assert!(
    waveform::MAX_AMPLITUDE_PIXELS < UI.microphone.left.canvas.height() as i32 / 2
);
const _: () = assert!(
    waveform::MAX_AMPLITUDE_PIXELS < UI.microphone.right.canvas.height() as i32 / 2
);
const _: () = assert!(UI.imu.header_columns.roll_x < CONTENT_WIDTH);
const _: () = assert!(UI.imu.header_columns.pitch_x < CONTENT_WIDTH);
const _: () = assert!(UI.imu.header_columns.yaw_x < CONTENT_WIDTH);
