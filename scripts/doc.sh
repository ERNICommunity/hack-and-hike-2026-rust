#!/usr/bin/env bash
# Build the documentation of the library and serve it at http://localhost:8000.
#
# `cargo doc --open` cannot start a browser from inside the dev container. So
# this script serves the pages with a small web server. VS Code forwards the
# port to your computer. The script gives extra arguments to `cargo doc`, for
# example `--document-private-items`. Set PORT to use another port.
set -euo pipefail
cd "$(dirname "$0")/.."
port="${PORT:-8000}"
cargo doc "$@"
target_dir="$(cargo metadata --format-version 1 --no-deps |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
doc_dir="$target_dir/xtensa-esp32s3-none-elf/doc"
# rustdoc writes no page for the top URL, but VS Code opens that URL. This
# page redirects to the documentation of the library.
echo '<meta http-equiv="refresh" content="0; url=hack_and_hike/">' > "$doc_dir/index.html"
echo "Documentation: http://localhost:$port/hack_and_hike/"
echo "Stop the server with Ctrl+C."
exec python3 -m http.server "$port" --bind 127.0.0.1 --directory "$doc_dir"
