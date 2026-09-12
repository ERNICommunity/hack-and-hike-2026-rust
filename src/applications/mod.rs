//! Build-time application selection.
//!
//! Exactly one application owns the enabled capability handles and decides how
//! to combine them. Applications may be graphical or headless: omitting the
//! `display` capability is sufficient for a headless application.
//!
//! `app-stock` selects the normal Hack & Hike firmware. `app-idle` is a minimal
//! headless application useful for capability-only builds. If no application
//! feature is selected, the same idle application is used as a fallback so each
//! hardware capability can still be compiled independently.

#[cfg(all(feature = "app-stock", feature = "app-idle"))]
compile_error!("select exactly one application feature: `app-stock` or `app-idle`");

#[cfg(feature = "app-stock")]
mod stock;

use embassy_executor::Spawner;
#[cfg(not(feature = "app-stock"))]
use embassy_time::{Duration, Timer};

use crate::firmware::Bootstrap;

#[cfg(feature = "app-stock")]
pub(crate) async fn run(spawner: Spawner, bootstrap: Bootstrap) -> ! {
    stock::run(spawner, bootstrap).await
}

#[cfg(not(feature = "app-stock"))]
pub(crate) async fn run(_spawner: Spawner, bootstrap: Bootstrap) -> ! {
    // Keep ownership of every enabled application-facing endpoint. CPU1
    // capability runtimes continue running even though this application does no
    // foreground work and may not have a display at all.
    let _bootstrap = bootstrap;
    loop {
        Timer::after(Duration::from_millis(100)).await;
    }
}
