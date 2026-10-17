#!/bin/sh
set -eu

phase=${1:-}
case "$phase" in
  seed|build) ;;
  *) printf 'Usage: build-api.sh seed|build\n' >&2; exit 2 ;;
esac

binaries='panda-api panda-restore-agent rebaseline_db panda-keys'
target=
if [ "${TARGETARCH:-}" = arm64 ] && [ "${BUILDARCH:-}" != arm64 ]; then
  target=aarch64-unknown-linux-gnu
  export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
  export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
  export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
  export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
  export PKG_CONFIG_ALLOW_CROSS_aarch64_unknown_linux_gnu=1
  export PKG_CONFIG_PATH_aarch64_unknown_linux_gnu=/usr/lib/aarch64-linux-gnu/pkgconfig
  export AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_LIB_DIR=/usr/lib/aarch64-linux-gnu
  export AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR=/usr/include
fi

if [ "$phase" = seed ]; then
  mkdir -p src/bin
  printf 'pub fn __panda_dummy() {}\n' > src/lib.rs
  printf 'fn main() {}\n' > src/main.rs
  for binary in $binaries; do
    [ "$binary" = panda-api ] || printf 'fn main() {}\n' > "src/bin/$binary.rs"
  done
else
  touch src/lib.rs src/main.rs migrations/0001_initial_schema.sql
  for binary in $binaries; do
    [ "$binary" = panda-api ] || touch "src/bin/$binary.rs"
  done
fi

set -- --locked --profile container --jobs "${CARGO_BUILD_JOBS:-$(nproc)}"
artifact_directory=target/container
if [ -n "$target" ]; then
  set -- "$@" --target "$target"
  artifact_directory="target/$target/container"
fi
for binary in $binaries; do
  set -- "$@" --bin "$binary"
done
cargo build "$@"

if [ "$phase" = seed ]; then
  rm -rf src
else
  for binary in $binaries; do
    cp "$artifact_directory/$binary" "./$binary"
  done
fi
