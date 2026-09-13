//! `embedded-gui` styles in the firmware palette.
//!
//! KDL screens reference them as `style="crate::styles::title()"`; the demo
//! re-exports this module under that name.

use embedded_gui::{FontId, Style, WidgetStyle};

use super::{common, theme};

const BUTTON_CORNER_RADIUS: u8 = 4;

/// Screen title.
pub const fn title() -> Style {
    let mut style = Style::label();
    style.font = FontId::MonoFont(common::TITLE_FONT);
    style.text = theme::DARK_BLUE;
    style
}

/// Regular text.
pub const fn body() -> Style {
    let mut style = Style::label();
    style.font = FontId::MonoFont(common::BODY_FONT);
    style.text = theme::CHARCOAL;
    style
}

/// De-emphasized text such as instructions.
pub const fn hint() -> Style {
    let mut style = body();
    style.text = theme::DARK_GRAY;
    style
}

/// A `value_label`: caption on the left, number on the right.
pub const fn value() -> Style {
    let mut style = body();
    style.accent = theme::DARK_BLUE;
    style
}

/// The main action on a screen.
pub const fn button() -> WidgetStyle {
    filled_button(theme::DARK_BLUE, theme::WHITE, theme::LIGHT_BLUE)
}

/// A less prominent action.
pub const fn secondary_button() -> WidgetStyle {
    filled_button(theme::LIGHT_GRAY, theme::CHARCOAL, theme::DARK_GRAY)
}

const fn filled_button(
    background: embedded_graphics::pixelcolor::Rgb565,
    text: embedded_graphics::pixelcolor::Rgb565,
    pressed_background: embedded_graphics::pixelcolor::Rgb565,
) -> WidgetStyle {
    let mut normal = Style::button();
    normal.background = Some(background);
    normal.gradient = None;
    normal.shadow = None;
    normal.text = text;
    normal.foreground = text;
    normal.font = FontId::MonoFont(common::BODY_FONT);
    normal.corner_radius = BUTTON_CORNER_RADIUS;

    let mut pressed = normal;
    pressed.background = Some(pressed_background);

    let mut style = WidgetStyle::new(normal);
    // No focus ring: the panel has no keyboard, so focus means nothing here.
    style.focused = normal;
    style.pressed = pressed;
    style
}
