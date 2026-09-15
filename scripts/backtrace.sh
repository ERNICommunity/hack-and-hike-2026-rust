#!/usr/bin/env bash
# Show the function, file and line of each address in a panic backtrace.
#
# The panic handler prints the backtrace as bare addresses. The firmware on the
# board has no debug information, but the ELF file of the build has it. `cargo
# dist` leaves that ELF file in the Cargo target directory. addr2line looks up
# each address in it. This script works only for the ESP32-S3.
#
# Usage: ./scripts/backtrace.sh <app> [log file]
#
# Paste the panic output from the serial log and press Ctrl+D. Or give a file
# that contains it. Use the build that is on the board: a new build moves the
# addresses, and the output is then wrong.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "Usage: $0 <app> [log file]" >&2
    exit 2
fi
app="$1"
input="${2:-/dev/stdin}"

target_dir="$(cargo metadata --format-version 1 --no-deps |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
elf="$target_dir/xtensa-esp32s3-none-elf/release/$app"
if [ ! -f "$elf" ]; then
    echo "No release build of '$app' at $elf." >&2
    echo "Build it with: cargo dist --bin $app" >&2
    exit 1
fi

# addr2line is part of the ESP toolchain. Often only interactive shells have
# it on their PATH, so load export-esp.sh when it is missing.
addr2line=xtensa-esp32s3-elf-addr2line
if ! command -v "$addr2line" > /dev/null && [ -f "$HOME/export-esp.sh" ]; then
    # shellcheck source=/dev/null
    source "$HOME/export-esp.sh"
fi

if [ $# -eq 1 ] && [ -t 0 ]; then
    echo "Paste the panic output, then press Ctrl+D:" >&2
fi

# On the ESP32-S3, code is between 0x40000000 and 0x43ffffff (ROM, internal
# RAM and flash). Other numbers in the log are data, not code addresses.
mapfile -t addresses < <(grep -oE '0x4[0-3][0-9a-fA-F]{6}\b' "$input" || true)
if [ ${#addresses[@]} -eq 0 ]; then
    echo "No code addresses found in the input." >&2
    exit 1
fi

echo >&2
"$addr2line" --exe "$elf" --pretty-print --functions --inlines --demangle --addresses "${addresses[@]}"
