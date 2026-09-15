#!/usr/bin/env bash
# Turn the backtrace of a panic into function names, files and lines.
#
# The panic handler prints the stack as bare addresses: the firmware on the
# board carries no debug information. The ELF file that `cargo dist` builds
# next to firmware.bin does, and addr2line looks each address up in it.
#
# Usage: ./scripts/backtrace.sh <app> [log file]
#
# Paste the panic output from the serial log and press Ctrl+D, or pass a file
# that holds it. Decode with the build you flashed: rebuilding the application
# moves the addresses, and the lines printed would be wrong.
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

# The Xtensa binutils come with the ESP toolchain, which only interactive
# shells have on their PATH.
addr2line=xtensa-esp32s3-elf-addr2line
if ! command -v "$addr2line" > /dev/null && [ -f "$HOME/export-esp.sh" ]; then
    # shellcheck source=/dev/null
    source "$HOME/export-esp.sh"
fi

if [ $# -eq 1 ] && [ -t 0 ]; then
    echo "Paste the panic output, then press Ctrl+D:" >&2
fi

# Code runs from 0x40000000 to 0x43ffffff (internal RAM and flash); other
# numbers in the log are data, not return addresses.
mapfile -t addresses < <(grep -oE '0x4[0-3][0-9a-fA-F]{6}\b' "$input" || true)
if [ ${#addresses[@]} -eq 0 ]; then
    echo "No code addresses found in the input." >&2
    exit 1
fi

echo >&2
"$addr2line" --exe "$elf" --pretty-print --functions --inlines --demangle --addresses "${addresses[@]}"
