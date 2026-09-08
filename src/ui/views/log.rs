//! Device log view.

use super::{
    common,
    super::{design, framebuffer::ContentFramebuffer},
};

pub(super) fn render_shell(frame: &mut ContentFramebuffer) {
    frame.clear(design::UI.content_background);
}

pub(super) fn render(frame: &mut ContentFramebuffer, text: &str) {
    common::render_text_page(frame, trailing_lines(text, design::UI.text.visible_lines));
}

fn trailing_lines(text: &str, line_count: usize) -> &str {
    if line_count == 0 || text.is_empty() {
        return "";
    }

    let bytes = text.as_bytes();
    let mut seen = 0usize;
    for index in (0..bytes.len()).rev() {
        if bytes[index] != b'\n' {
            continue;
        }

        seen += 1;
        if seen > line_count {
            return &text[index + 1..];
        }
    }

    text
}
