//! Build-time application selection.
//!
//! Exactly one application owns the enabled capability handles and decides how
//! to combine them. Applications may be graphical or headless: omitting the
//! `display` capability is sufficient for a headless application.
//!
//! `app-demo` selects the normal Hack & Hike demo firmware. `app-imu-color` and
//! `app-color-ping` are small example applications. `app-idle` is a minimal
//! headless application useful for capability-only builds. If no application
//! feature is selected, the same idle behavior is used as a fallback.

#[cfg(any(
    all(feature = "app-demo", feature = "app-idle"),
    all(feature = "app-demo", feature = "app-imu-color"),
    all(feature = "app-demo", feature = "app-color-ping"),
    all(feature = "app-idle", feature = "app-imu-color"),
    all(feature = "app-idle", feature = "app-color-ping"),
    all(feature = "app-imu-color", feature = "app-color-ping"),
))]
compile_error!(
    "select exactly one application feature: `app-demo`, `app-imu-color`, `app-color-ping`, or `app-idle`"
);

#[cfg(feature = "app-color-ping")]
mod color_ping;
#[cfg(feature = "app-demo")]
mod demo;
#[cfg(feature = "app-imu-color")]
mod imu_color;

use embassy_executor::Spawner;
#[cfg(not(any(
    feature = "app-demo",
    feature = "app-imu-color",
    feature = "app-color-ping",
)))]
use embassy_time::{Duration, Timer};

use crate::firmware::Bootstrap;

#[cfg(feature = "app-demo")]
pub(crate) async fn run(spawner: Spawner, bootstrap: Bootstrap) -> ! {
    demo::run(spawner, bootstrap).await
}

#[cfg(feature = "app-imu-color")]
pub(crate) async fn run(spawner: Spawner, bootstrap: Bootstrap) -> ! {
    imu_color::run(spawner, bootstrap).await
}

#[cfg(feature = "app-color-ping")]
pub(crate) async fn run(spawner: Spawner, bootstrap: Bootstrap) -> ! {
    color_ping::run(spawner, bootstrap).await
}

#[cfg(not(any(
    feature = "app-demo",
    feature = "app-imu-color",
    feature = "app-color-ping",
)))]
pub(crate) async fn run(_spawner: Spawner, bootstrap: Bootstrap) -> ! {
    // Keep ownership of every enabled application-facing endpoint. CPU1
    // capability runtimes continue running even though this application does no
    // foreground work and may not have a display at all.
    let _bootstrap = bootstrap;
    loop {
        Timer::after(Duration::from_millis(100)).await;
    }
}
