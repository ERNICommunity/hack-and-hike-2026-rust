#!/usr/bin/env bash
# Build the library's documentation and serve it at http://localhost:8000.
#
# `cargo doc --open` cannot start a browser from inside the devcontainer, so
# this serves the generated pages with a small web server instead; VS Code
# forwards the port to your computer. Extra arguments go to `cargo doc`, for
# example `--document-private-items`. Set PORT to use another port.
set -euo pipefail
cd "$(dirname "$0")/.."
port="${PORT:-8000}"
cargo doc "$@"
target_dir="$(cargo metadata --format-version 1 --no-deps |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
doc_dir="$target_dir/xtensa-esp32s3-none-elf/doc"
# rustdoc writes no page for the root, which VS Code opens; send it to the library.
echo '<meta http-equiv="refresh" content="0; url=hack_and_hike/">' > "$doc_dir/index.html"
echo "Documentation: http://localhost:$port/hack_and_hike/"
echo "Stop the server with Ctrl+C."
exec python3 -m http.server "$port" --bind 127.0.0.1 --directory "$doc_dir"
