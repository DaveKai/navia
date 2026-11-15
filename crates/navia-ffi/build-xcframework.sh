#!/usr/bin/env bash
# Builds Navia's core as an xcframework with Swift bindings. Run on a Mac with the needed Rust targets. Output goes to
# build/ here.
#
# crates/navia-ffi/build-xcframework.sh [arm64|macos|all]   (default: arm64)
#
# arm64 is Apple Silicon only, macos adds x86_64, all adds iOS and the simulator. Runs at low priority on 2 jobs and
# skips if another build holds the lock.
set -euo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

LOCK=/tmp/navia-mac-build.lock
if ! mkdir "$LOCK" 2>/dev/null; then
  echo "another Navia build holds $LOCK; try again later" >&2
  exit 75
fi
trap 'rmdir "$LOCK"' EXIT

export CARGO_TARGET_DIR="$PWD/target"
export MACOSX_DEPLOYMENT_TARGET=14.2
export IPHONEOS_DEPLOYMENT_TARGET=17.0
OUT=build
JOBS=2
rm -rf "$OUT"
mkdir -p "$OUT/headers" "$OUT/swift" "$OUT/lib"

SCOPE="${1:-arm64}"
TARGETS=(aarch64-apple-darwin)
case "$SCOPE" in
  arm64) ;;
  macos) TARGETS+=(x86_64-apple-darwin) ;;
  all) TARGETS+=(x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim) ;;
  *) echo "usage: $0 [arm64|macos|all]" >&2; exit 2 ;;
esac

for t in "${TARGETS[@]}"; do
  echo "== building $t"
  nice -n 15 cargo build -j "$JOBS" --release --no-default-features --target "$t"
done

if [ "$SCOPE" = "arm64" ]; then
  cp target/aarch64-apple-darwin/release/libnavia_ffi.a "$OUT/lib/libnavia_ffi_macos.a"
else
  lipo -create target/aarch64-apple-darwin/release/libnavia_ffi.a target/x86_64-apple-darwin/release/libnavia_ffi.a -output "$OUT/lib/libnavia_ffi_macos.a"
fi

echo "== generating Swift bindings"
nice -n 15 cargo run -j "$JOBS" --release --no-default-features --features bindgen --bin uniffi-bindgen -- generate --library target/aarch64-apple-darwin/release/libnavia_ffi.dylib --language swift --out-dir "$OUT/gen"
cp "$OUT/gen/"*.swift "$OUT/swift/"
cp "$OUT/gen/"*.h "$OUT/headers/"
# xcodebuild looks for module.modulemap beside the headers.
cat "$OUT/gen/"*.modulemap > "$OUT/headers/module.modulemap"

CREATE_ARGS=(-library "$OUT/lib/libnavia_ffi_macos.a" -headers "$OUT/headers")
if [ "$SCOPE" = "all" ]; then
  CREATE_ARGS+=(-library target/aarch64-apple-ios/release/libnavia_ffi.a -headers "$OUT/headers")
  CREATE_ARGS+=(-library target/aarch64-apple-ios-sim/release/libnavia_ffi.a -headers "$OUT/headers")
fi

nice -n 15 xcodebuild -create-xcframework "${CREATE_ARGS[@]}" -output "$OUT/NaviaCoreFFI.xcframework"
echo "== done: $OUT/NaviaCoreFFI.xcframework and $OUT/swift/"
ls -la "$OUT/NaviaCoreFFI.xcframework" "$OUT/swift"
