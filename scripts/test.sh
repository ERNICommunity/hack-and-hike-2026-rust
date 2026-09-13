#!/usr/bin/env bash
# Run the host tests of crates/core.
#
# The repository's Cargo configuration targets the ESP32-S3, so a plain
# `cargo test` would try to build tests for the microcontroller. This runs
# the tests for your computer's target with the host toolchain instead.
set -euo pipefail
cd "$(dirname "$0")/.."
rust_version="$(sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml)"
host_target="$(rustc +"$rust_version" -vV | sed -n 's/^host: //p')"
exec cargo +"$rust_version" test -p hack-and-hike-core --target "$host_target" "$@"
