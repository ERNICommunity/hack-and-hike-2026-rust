#!/usr/bin/env bash
# Runs once after the container is built.
# See the output in VS Code via "Dev Containers: Show Container Log".
set -euo pipefail

# Download all dependencies now so the first build does not start with a
# multi-minute git clone of the esp-hal repository.
cargo fetch
