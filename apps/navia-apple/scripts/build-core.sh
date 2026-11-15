#!/usr/bin/env bash
# Builds the Rust core for the Apple Silicon Mac and installs it where Xcode links it from.
# Runs on the Mac. Uses the shared build lock through build-xcframework.sh.
#   apps/navia-apple/scripts/build-core.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
FFI="$ROOT/crates/navia-ffi"
KIT="$ROOT/apps/navia-apple/NaviaCoreKit"
"$FFI/build-xcframework.sh" arm64
rm -rf "$KIT/navia_ffiFFI.xcframework"
cp -R "$FFI/build/NaviaCoreFFI.xcframework" "$KIT/navia_ffiFFI.xcframework"
mkdir -p "$KIT/Sources/NaviaCoreFFI"
cp "$FFI/build/swift/navia_ffi.swift" "$KIT/Sources/NaviaCoreFFI/navia_ffi.swift"
echo "== linked into $KIT"
