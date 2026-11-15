#!/usr/bin/env bash
# Compiles the generated Swift against the built xcframework and runs the smoke test. Run on a Mac.
set -euo pipefail
cd "$(dirname "$0")"
LOCK=/tmp/navia-mac-build.lock
if ! mkdir "$LOCK" 2>/dev/null; then
  echo "another Navia build holds $LOCK; try again later" >&2
  exit 75
fi
trap 'rmdir "$LOCK"' EXIT
SLICE=build/NaviaCoreFFI.xcframework/macos-arm64
mkdir -p build/smoke build/smoke-data
nice -n 15 swiftc -swift-version 5 -j2 -I "$SLICE/Headers" -L "$SLICE" -lnavia_ffi_macos \
  -framework CoreAudio -framework AudioToolbox -framework AudioUnit -framework CoreFoundation -framework Security -framework SystemConfiguration -lc++ \
  build/swift/navia_ffi.swift smoke/main.swift -o build/smoke/smoke
./build/smoke/smoke
