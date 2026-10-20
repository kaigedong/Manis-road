#!/usr/bin/env bash
# Build the Rust JNI library into android/app/src/main/jniLibs.
# Usage: ./android/build-rust.sh [debug|release]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Android/sdk}}"
NDK_VERSION="${NDK_VERSION:-27.0.12077973}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/$NDK_VERSION}"

ABI="${ABI:-arm64-v8a}"
OUT="$ROOT/android/app/src/main/jniLibs"
PROFILE="${1:-debug}"

if [[ ! -d "$ANDROID_NDK_HOME" ]]; then
  echo "Android NDK not found at $ANDROID_NDK_HOME" >&2
  echo "Set ANDROID_NDK_HOME or install NDK $NDK_VERSION." >&2
  exit 1
fi

mkdir -p "$OUT"
cd "$ROOT"

if [[ "$PROFILE" == "release" ]]; then
  cargo ndk -t "$ABI" -o "$OUT" build -p manis-motu-ffi --release
else
  cargo ndk -t "$ABI" -o "$OUT" build -p manis-motu-ffi
fi

ls -l "$OUT/$ABI/libmotu_ffi.so"
