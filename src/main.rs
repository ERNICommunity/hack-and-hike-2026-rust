#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod applications;
mod capabilities;
mod firmware;
mod platform;
mod support;
#[cfg(feature = "ui")]
mod ui;

extern crate alloc;

use embassy_executor::Spawner;
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(cpu0_spawner: Spawner) -> ! {
    let bootstrap = firmware::bootstrap();
    applications::run(cpu0_spawner, bootstrap).await
}
