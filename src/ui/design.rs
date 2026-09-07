//! Declarative, compile-time presentation specification.
//!
//! This module is the editable design surface for the firmware UI. Geometry and
//! semantic color assignments are plain `const` Rust data: renderers consume the
//! specs but do not own sizing or styling policy. This keeps manual changes easy
//! to review while preserving zero-allocation, monomorphized drawing code.
//!
//! `ContentRect` is deliberately distinct from `display::Region`: content-space
//! coordinates are relative to the framebuffer, while `display::Region` uses
//! physical panel coordinates. Converting between them is explicit.

use crate::{
    display::{self, Region},
    theme,
};

const NAV_WIDTH: usize = 44;
const NAV_BUTTON_HEIGHT: usize = 48;

pub(crate) const CONTENT_WIDTH: usize = display::WIDTH - NAV_WIDTH;
pub(crate) const CONTENT_HEIGHT: usize = display::HEIGHT;
pub(crate) const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub(crate) const CONTENT_REGION: Region =
    Region::new(NAV_WIDTH, 0, CONTENT_WIDTH, CONTENT_HEIGHT);

/// Semantic UI color stored in the display's native RGB565 representation.
///
/// Keeping this distinct from bare integers prevents dimensions, counters, and
/// pixel values from being accidentally interchanged in presentation code.
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

/// Rectangle in content-framebuffer coordinates, not physical screen space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl ContentRect {
    pub(crate) const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
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

/// Fixed navigation-rail geometry and semantic colors.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NavigationSpec {
    pub width: usize,
    pub button_height: usize,
    pub selected_background: UiColor,
    pub normal_background: UiColor,
    pub selected_icon: UiColor,
    pub normal_icon: UiColor,
    pub divider: UiColor,
}

/// Shared monospaced page layout used by Network and Log.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextPageSpec {
    pub x: i32,
    pub top: i32,
    pub line_height: i32,
    pub visible_lines: usize,
    pub foreground: UiColor,
    pub background: UiColor,
}

/// Centered placeholder page styling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlaceholderSpec {
    pub title_y: i32,
    pub subtitle_y: i32,
    pub title: UiColor,
    pub subtitle: UiColor,
    pub background: UiColor,
}

/// One microphone panel in content coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaveformPanelSpec {
    pub label: &'static str,
    pub panel: ContentRect,
    pub label_x_offset: i32,
    pub label_y_offset: i32,
    pub canvas: ContentRect,
}

/// Static microphone chrome and high-rate waveform styling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MicrophoneSpec {
    pub panels: [WaveformPanelSpec; 2],
    pub panel_fill: UiColor,
    pub panel_border: UiColor,
    pub label: UiColor,
    pub canvas_background: UiColor,
    pub grid: UiColor,
    pub trace: UiColor,
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
    pub header_value_x: [i32; 3],
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
        panels: [
            WaveformPanelSpec {
                label: "MIC L",
                panel: ContentRect::new(4, 4, CONTENT_WIDTH - 8, 114),
                label_x_offset: 6,
                label_y_offset: 4,
                canvas: ContentRect::new(10, 22, 256, 90),
            },
            WaveformPanelSpec {
                label: "MIC R",
                panel: ContentRect::new(4, 122, CONTENT_WIDTH - 8, 114),
                label_x_offset: 6,
                label_y_offset: 4,
                canvas: ContentRect::new(10, 140, 256, 90),
            },
        ],
        panel_fill: BLACK,
        panel_border: DARK_GRAY,
        label: LIGHT_GRAY,
        canvas_background: WHITE,
        grid: LIGHT_GRAY,
        trace: DARK_BLUE,
    },
    imu: ImuSpec {
        header: ContentRect::new(6, 6, CONTENT_WIDTH - 12, 44),
        attitude: ContentRect::new(6, 56, CONTENT_WIDTH - 12, 140),
        compass: ContentRect::new(6, 202, CONTENT_WIDTH - 12, 32),
        header_value_x: [83, 143, 207],
        background: WHITE,
        primary: DARK_BLUE,
        horizon_sky: LIGHT_BLUE,
        secondary: DARK_GRAY,
        border: LIGHT_GRAY,
        on_primary: WHITE,
    },
};

const _: () = assert!(NAV_WIDTH < display::WIDTH);
const _: () = assert!(NAV_BUTTON_HEIGHT * 5 == display::HEIGHT);
const _: () = assert!(UI.microphone.panels[0].canvas.x + UI.microphone.panels[0].canvas.width <= CONTENT_WIDTH);
const _: () = assert!(UI.microphone.panels[1].canvas.x + UI.microphone.panels[1].canvas.width <= CONTENT_WIDTH);
const _: () = assert!(UI.microphone.panels[0].canvas.y + UI.microphone.panels[0].canvas.height <= CONTENT_HEIGHT);
const _: () = assert!(UI.microphone.panels[1].canvas.y + UI.microphone.panels[1].canvas.height <= CONTENT_HEIGHT);
