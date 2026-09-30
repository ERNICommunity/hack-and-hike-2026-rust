#!/usr/bin/env bash
# Reproduces the Xtensa bug with bytes of a global's address: correct code
# up to LLVM 20, a crash ("Cannot select PCREL_WRAPPER") in upstream LLVM 21+
# and Espressif LLVM 21.1.3, wrong code in Espressif LLVM 22.1.4 and esp-rs
# Rust 1.98.1.0. Run setup.sh first (and build_upstream_llc.sh for the
# upstream llc, or point UPSTREAM_LLC at one).
set -uo pipefail
cd "$(dirname "$0")"

if [ -z "${UPSTREAM_LLC:-}" ] && [ -x /opt/llvm-upstream-xtensa/bin/llc ]; then
  UPSTREAM_LLC=/opt/llvm-upstream-xtensa/bin/llc
fi

ESP=/opt/esp-llvm
LLVM20=$ESP/20.1.1_20250829/bin
LLVM21=$ESP/21.1.3_20260408/bin
LLVM22=$ESP/22.1.4_20260825/bin
LLCS=("$LLVM20/llc" "$LLVM21/llc" "$LLVM22/llc" ${UPSTREAM_LLC:+"$UPSTREAM_LLC"})
FILECHECK=/usr/lib/llvm-21/bin/FileCheck
QEMU=/opt/qemu-xtensa/usr/bin/qemu-xtensa
GNU=$(ls -d /usr/local/rustup/toolchains/esp/xtensa-esp-elf/*/xtensa-esp-elf 2>/dev/null | tail -1)
export XTENSA_GNU_CONFIG=$GNU/lib/xtensa_esp32s3.so   # little-endian ESP32-S3 for as/ld/gcc

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# The crash, or per function "ok" or the load that reads memory at the
# global (l8ui/l16ui/l16si after the l32r of its address).
verdict() {
  local log
  if log=$("$@" 2>&1); then
    echo "$log" | awk '
      /^[A-Za-z_][A-Za-z0-9_]*:/ { if (f != "") print "    " f ": " (bad ? bad : "ok"); f = $1; sub(":", "", f); bad = "" }
      /^[ \t]+(l8ui|l16ui|l16si)[ \t]/ { if (bad == "") { bad = $0; gsub(/[ \t]+/, " ", bad); sub(/^ /, "", bad); bad = "WRONG: " bad } }
      END { if (f != "") print "    " f ": " (bad ? bad : "ok") }' | grep -v -E '^[[:space:]]+\.'
  else
    echo "$log" | grep -m1 -oE '(LLVM ERROR|error in backend): Cannot select: .*PCREL_WRAPPER|(LLVM ERROR|error in backend): Cannot select: t[0-9]+: i32 = <<Unknown Target Node #[0-9]+>> TargetConstantPool' | sed 's/^/    CRASH: /'
  fi
}

name() { "$1" --version | grep -m1 -iE 'llvm version|clang version' | sed 's/^ *//; s/ (http.*//'; }

echo "== llc -mtriple=xtensa -mcpu=esp32s3 -O2 low_byte.ll"
for llc in "${LLCS[@]}"; do
  echo "  $(name "$llc")"
  verdict "$llc" -mtriple=xtensa -mcpu=esp32s3 -O2 low_byte.ll -o -
done

echo
echo "== FileCheck (the RUN lines of low_byte.ll)"
for llc in "${LLCS[@]}"; do
  printf '  %-34s' "$(name "$llc")"
  for flags in "-O2" "-mcpu=esp32s3 -O2" "-mcpu=esp32s3 -O0"; do
    # shellcheck disable=SC2086
    if "$llc" -mtriple=xtensa $flags low_byte.ll -o - 2>/dev/null | "$FILECHECK" low_byte.ll >/dev/null 2>&1; then r=pass; else r=FAIL; fi
    printf '  [%s] %s' "$flags" "$r"
  done
  echo
done

echo
echo "== clang --target=xtensa-esp-elf -mcpu=esp32s3 -O2 -S low_byte.c"
for bin in "$LLVM21" "$LLVM22"; do
  echo "  $(name "$bin/clang")"
  verdict "$bin/clang" --target=xtensa-esp-elf -mcpu=esp32s3 -O2 -S low_byte.c -o -
done
rm -f /tmp/low_byte-*.c /tmp/low_byte-*.sh   # clang's crash reproducers

# esp-rs Rust: the installed esp channel (1.97.0.0 here) on the no_std crate,
# and 1.93.0.0 / 1.98.1.0 from setup.sh on the same function without core.
rust_no_core() {
  RUSTC_BOOTSTRAP=1 "$@" --target xtensa-esp32s3-none-elf -C opt-level=3 -C panic=abort rust/low_byte_no_core.rs
}
RUST193=()
[ -x /opt/esp-rust-1.93.0.0/bin/rustc ] && RUST193=(/opt/esp-rust-1.93.0.0/bin/rustc)
RUST198=()
if [ -x /opt/esp-rust-1.98.1.0-x86_64/bin/rustc ]; then
  RUST198=(/opt/esp-rust-1.98.1.0-x86_64/bin/rustc)
  [ "$(uname -m)" = aarch64 ] && RUST198=(/opt/qemu-x86_64/usr/bin/qemu-x86_64 "${RUST198[@]}")
fi

echo
echo "== rustc, release"
if [ ${#RUST193[@]} -gt 0 ]; then
  echo "  $("${RUST193[@]}" --version) (rust/low_byte_no_core.rs)"
  verdict rust_no_core "${RUST193[@]}" --emit asm -o -
fi
if command -v cargo >/dev/null && [ -d /usr/local/rustup/toolchains/esp ]; then
  echo "  $(rustc +esp --version) (rust/, no LTO)"
  log=$(cd rust && CARGO_TARGET_DIR="$out/rust" cargo +esp rustc -q --release -- --emit asm 2>&1)
  if echo "$log" | grep -q 'Cannot select'; then
    echo "$log" | grep -m1 -oE 'LLVM ERROR: Cannot select: .*PCREL_WRAPPER' | sed 's/^/    CRASH: /'
  else
    verdict cat "$out"/rust/xtensa-esp32s3-none-elf/release/deps/low_byte-*.s
  fi
fi
if [ ${#RUST198[@]} -gt 0 ]; then
  echo "  $("${RUST198[@]}" --version) (rust/low_byte_no_core.rs)"
  verdict rust_no_core "${RUST198[@]}" --emit asm -o -
fi

echo
echo "== Running low_byte() under qemu-xtensa: g is at 0x00500010, g[0] = 0xAB"
"$GNU/bin/xtensa-esp-elf-as" runtime/harness.s -o "$out/harness.o"
"$GNU/bin/xtensa-esp-elf-gcc" -O2 -c low_byte.c -o "$out/gcc.o"
"$LLVM20/llc" -mtriple=xtensa -mcpu=esp32s3 -O2 -filetype=obj low_byte.ll -o "$out/llvm-20.1.1.o"
"$LLVM22/llc" -mtriple=xtensa -mcpu=esp32s3 -O2 -filetype=obj low_byte.ll -o "$out/llvm-22.1.4.o"
names=(gcc llvm-20.1.1 llvm-22.1.4)
if [ ${#RUST198[@]} -gt 0 ]; then
  rust_no_core "${RUST198[@]}" --emit obj -o "$out/rust-1.98.1.o"
  names+=(rust-1.98.1)
fi
for n in "${names[@]}"; do
  "$GNU/bin/xtensa-esp-elf-ld" -T runtime/link.ld "$out/harness.o" "$out/$n.o" -o "$out/$n.elf" 2>/dev/null
  "$QEMU" -cpu dc233c "$out/$n.elf"
  status=$?
  if [ "$status" -eq 16 ]; then v="correct"; else v="WRONG, expected 0x10"; fi
  printf '  %-12s returns 0x%02x: %s\n' "$n" "$status" "$v"
done
