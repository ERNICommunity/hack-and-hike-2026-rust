//! Stock application-specific view implementations.
//!
//! Generic shell/navigation/design/GUI hosting lives in `crate::ui`. These
//! re-exports keep view-local helper paths stable while the implementation is
//! owned by the top-level UI shell.

pub(crate) use crate::ui::{design, gui, navigation, theme};
pub(crate) mod views;
