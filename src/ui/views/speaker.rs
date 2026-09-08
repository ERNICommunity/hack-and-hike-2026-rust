//! Speaker output view.

use super::{common, super::framebuffer::ContentFramebuffer};

pub(super) fn render_shell(frame: &mut ContentFramebuffer) {
    common::render_placeholder(frame, "SPEAKER", "Speaker output");
}
