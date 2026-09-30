#!/usr/bin/env bash
# Builds llc from upstream llvm-project main with only the (experimental)
# Xtensa target, assertions on, into /opt/llvm-upstream-xtensa/bin/llc.
# About 10 minutes on 12 cores; the build tree (~900 MB) is removed after.
#   ./build_upstream_llc.sh [commit]
set -euo pipefail

commit=${1:-main}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

command -v cmake >/dev/null && command -v ninja >/dev/null \
  || sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends cmake ninja-build

git clone --filter=blob:none --no-checkout https://github.com/llvm/llvm-project.git "$work/src"
cd "$work/src"
git sparse-checkout init --no-cone
printf '%s\n' /cmake/ /third-party/ /llvm/ '!/llvm/test/' '!/llvm/unittests/' '!/llvm/docs/' '!/llvm/benchmarks/' \
  /libc/ '!/libc/test/' '!/libc/docs/' '!/libc/benchmarks/' '!/libc/fuzzing/' | git sparse-checkout set --no-cone --stdin
git checkout "$commit"

cmake -S llvm -B "$work/build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DLLVM_ENABLE_ASSERTIONS=ON \
  -DLLVM_TARGETS_TO_BUILD="" -DLLVM_EXPERIMENTAL_TARGETS_TO_BUILD=Xtensa \
  -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_DOCS=OFF \
  -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF -DLLVM_ENABLE_TERMINFO=OFF
ninja -C "$work/build" llc

sudo mkdir -p /opt/llvm-upstream-xtensa/bin
sudo cp "$work/build/bin/llc" /opt/llvm-upstream-xtensa/bin/llc
echo "llvm-project $(git rev-parse HEAD) ($(git log -1 --format=%cs)), Release + assertions, Xtensa only" \
  | sudo tee /opt/llvm-upstream-xtensa/VERSION
