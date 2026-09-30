#!/usr/bin/env bash
# Installs what reproduce.sh needs, on Debian 13 (trixie), aarch64 or x86_64:
#   /opt/esp-llvm/<release>/bin/llc           Espressif LLVM 20.1.1, 21.1.3, 22.1.4
#   /opt/esp-llvm/<release>/bin/clang         Espressif clang 21.1.3 and 22.1.4
#   /opt/qemu-xtensa/usr/bin/qemu-xtensa      QEMU user mode for Xtensa
#   /usr/lib/llvm-21/bin/FileCheck            from apt.llvm.org
#   /opt/esp-rust-1.93.0.0/bin/rustc          esp-rs Rust 1.93.0.0 (LLVM 20), last good
#   /opt/esp-rust-1.98.1.0-x86_64/bin/rustc   esp-rs Rust 1.98.1.0, wrong code
# GNU binutils and GCC for Xtensa come from the esp Rust toolchain
# (espup installs them under /usr/local/rustup/toolchains/esp/xtensa-esp-elf).
set -euo pipefail

case "$(uname -m)" in
  aarch64) host=aarch64-linux-gnu ;;
  x86_64) host=x86_64-linux-gnu ;;
  *) echo "unsupported host $(uname -m)" >&2; exit 1 ;;
esac

# The last good release: llc only.
if [ ! -x /opt/esp-llvm/20.1.1_20250829/bin/llc ]; then
  sudo mkdir -p /opt/esp-llvm/20.1.1_20250829
  sudo chown "$(id -u):$(id -g)" /opt/esp-llvm/20.1.1_20250829
  curl -fsSL "https://github.com/espressif/llvm-project/releases/download/esp-20.1.1_20250829/clang-esp-20.1.1_20250829-$host.tar.xz" \
    | xz -dc | tar -x -C /opt/esp-llvm/20.1.1_20250829 --strip-components=1 --wildcards '*/bin/llc'
fi

for release in 21.1.3_20260408:21 22.1.4_20260825:22; do
  tag=${release%%:*}
  major=${release##*:}
  dir=/opt/esp-llvm/$tag
  if [ -x "$dir/bin/llc" ] && [ -x "$dir/bin/clang-$major" ]; then continue; fi
  sudo mkdir -p "$dir"
  sudo chown "$(id -u):$(id -g)" "$dir"
  # Only llc and clang (both statically linked) out of the ~400 MB archive.
  curl -fsSL "https://github.com/espressif/llvm-project/releases/download/esp-$tag/clang-esp-$tag-$host.tar.xz" \
    | xz -dc | tar -x -C "$dir" --strip-components=1 --wildcards '*/bin/llc' "*/bin/clang-$major" '*/bin/clang'
done

if [ ! -x /opt/qemu-xtensa/usr/bin/qemu-xtensa ]; then
  tmp=$(mktemp -d)
  (cd "$tmp" && apt-get download qemu-user)
  sudo mkdir -p /opt/qemu-xtensa
  dpkg-deb --fsys-tarfile "$tmp"/qemu-user_*.deb | sudo tar -x -C /opt/qemu-xtensa ./usr/bin/qemu-xtensa
  rm -rf "$tmp"
fi

if [ ! -x /usr/lib/llvm-21/bin/FileCheck ]; then
  sudo mkdir -p /etc/apt/keyrings
  curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key | sudo tee /etc/apt/keyrings/apt.llvm.org.asc >/dev/null
  echo "deb [signed-by=/etc/apt/keyrings/apt.llvm.org.asc] http://apt.llvm.org/trixie/ llvm-toolchain-trixie-21 main" \
    | sudo tee /etc/apt/sources.list.d/llvm-21.list >/dev/null
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends llvm-21-tools
fi

# esp-rs Rust 1.93.0.0 (LLVM 20.1.1), only its rustc.
rust193=/opt/esp-rust-1.93.0.0
if [ ! -x "$rust193/bin/rustc" ]; then
  sudo mkdir -p "$rust193"
  sudo chown "$(id -u):$(id -g)" "$rust193"
  curl -fsSL "https://github.com/esp-rs/rust-build/releases/download/v1.93.0.0/rust-1.93.0.0-$(uname -m)-unknown-linux-gnu.tar.xz" \
    | xz -dc | tar -x -C "$rust193" --strip-components=2 --wildcards '*/rustc/bin/rustc' '*/rustc/lib/*.so'
fi

# esp-rs Rust 1.98.1.0, only its rustc: there is no aarch64 Linux build of
# it, so on aarch64 the x86_64 build runs under qemu-x86_64.
rust198=/opt/esp-rust-1.98.1.0-x86_64
if [ ! -x "$rust198/bin/rustc" ]; then
  sudo mkdir -p "$rust198"
  sudo chown "$(id -u):$(id -g)" "$rust198"
  curl -fsSL https://github.com/esp-rs/rust-build/releases/download/v1.98.1.0/rust-1.98.1.0-x86_64-unknown-linux-gnu.tar.xz \
    | xz -dc | tar -x -C "$rust198" --strip-components=2 --wildcards '*/rustc/bin/rustc' '*/rustc/lib/*.so'
fi
if [ "$host" = aarch64-linux-gnu ] && [ ! -x /opt/qemu-x86_64/usr/bin/qemu-x86_64 ]; then
  tmp=$(mktemp -d)
  (cd "$tmp" && apt-get download qemu-user)
  sudo mkdir -p /opt/qemu-x86_64
  dpkg-deb --fsys-tarfile "$tmp"/qemu-user_*.deb | sudo tar -x -C /opt/qemu-x86_64 ./usr/bin/qemu-x86_64
  rm -rf "$tmp"
  sudo dpkg --add-architecture amd64
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
    libc6:amd64 libgcc-s1:amd64 libstdc++6:amd64 zlib1g:amd64
fi

echo "ok"
