#!/usr/bin/env bash
# Run the host tests of crates/core.
#
# The repository's Cargo configuration targets the ESP32-S3, so a plain
# `cargo test` would try to build tests for the microcontroller. This runs
# the tests for your computer's target with the host toolchain instead.
set -euo pipefail
cd "$(dirname "$0")/.."
host_target="$(rustc +1.97.0 -vV | sed -n 's/^host: //p')"
exec cargo +1.97.0 test -p hack-and-hike-core --target "$host_target" "$@"
