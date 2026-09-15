#!/usr/bin/env bash
# Run the tests of crates/core on your computer.
#
# The Cargo configuration of the repository builds for the ESP32-S3. So a
# plain `cargo test` tries to build the tests for the microcontroller. This
# script builds them for your computer instead. It uses the Rust version from
# `rust-version` in Cargo.toml, not the esp toolchain. Extra arguments go to
# `cargo test`.
set -euo pipefail
cd "$(dirname "$0")/.."
rust_version="$(sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml)"
host_target="$(rustc +"$rust_version" -vV | sed -n 's/^host: //p')"
exec cargo +"$rust_version" test -p hack-and-hike-core --target "$host_target" "$@"
