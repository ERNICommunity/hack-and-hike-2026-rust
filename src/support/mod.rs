#[cfg(any(
    feature = "touch",
    feature = "mic",
    feature = "speaker",
    feature = "network",
))]
pub(crate) mod diagnostics;
pub(crate) mod logging;
pub(crate) mod memory;
