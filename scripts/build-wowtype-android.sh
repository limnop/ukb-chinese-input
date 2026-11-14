#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "$0")/.." && pwd)"
if ! command -v rustup >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/rustup" ]]; then
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
fi
ndk_root="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "$ndk_root" ]]; then
  sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -n "$sdk_root" ]]; then
    ndk_root="$(find "$sdk_root/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)"
  fi
fi
if [[ -z "$ndk_root" || ! -d "$ndk_root/toolchains/llvm/prebuilt" ]]; then
  echo "Android NDK not found; set ANDROID_NDK_HOME" >&2
  exit 1
fi
if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to install Android Rust targets (aarch64-linux-android, armv7-linux-androideabi, i686-linux-android, x86_64-linux-android)" >&2
  exit 1
fi

host_tag="$(find "$ndk_root/toolchains/llvm/prebuilt" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
toolchain="$host_tag/bin"
targets=(aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android)
rustup target add --toolchain stable "${targets[@]}"
export CARGO_TARGET_DIR="$project_root/build/wowtype-rust"

for target in "${targets[@]}"; do
  case "$target" in
    aarch64-linux-android) linker="$toolchain/aarch64-linux-android21-clang" ;;
    armv7-linux-androideabi) linker="$toolchain/armv7a-linux-androideabi21-clang" ;;
    i686-linux-android) linker="$toolchain/i686-linux-android21-clang" ;;
    x86_64-linux-android) linker="$toolchain/x86_64-linux-android21-clang" ;;
  esac
  target_key="${target//-/_}"
  env "CARGO_TARGET_${target_key^^}_LINKER=$linker" \
    rustup run stable cargo build --manifest-path "$project_root/vendor/wowtype-core/Cargo.toml" \
      --target "$target" --release --no-default-features --lib
done
