#!/usr/bin/env bash
# Cross-compiles the GPUI app for Windows on Linux and packs a per-user Velopack installer.
#
#   packaging/windows/build-installer.sh [--arch x64|arm64] [--zip] [--releases DIR]
#
# Writes ~/Desktop/Navia-Setup-<version>-<arch>.exe. --zip also copies the portable zip next to it.
# --releases copies everything Velopack made into DIR: the feed (releases.<channel>.json and the
# nupkg) to upload where NAVIA_UPDATE_FEED points, plus Setup and the portable zip.
# Each architecture is its own channel, win-x64 or win-arm64, so neither updates to the other.
#
# Needs, under $NAVIA_WINTC (default ~/.local/share/navia-wintc), rustup/ and cargo/ with the
# x86_64-pc-windows-msvc or aarch64-pc-windows-msvc target and cargo-xwin, venv/bin with cmake, dotnet/ with the .NET SDK,
# and vpk/ at the version the velopack crate pins. Also clang-cl, lld-link, llvm-rc and the
# vendored GPUI (vendor/gpui/fetch-gpui.sh). Nothing is uploaded.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD
WINTC=${NAVIA_WINTC:-$HOME/.local/share/navia-wintc}
# Test builds are 0.1.0+<commit count>, which Velopack orders after 0.1.0 and older test builds,
# so a newer one installs as an update. A tagged release is 0.1.0.
VERSION=$(tools/version.sh full)
OUT_DIR=${NAVIA_OUT_DIR:-$HOME/Desktop}
ZIP=0
ARCH=x64
RELEASES=
while [ $# -gt 0 ]; do
    case "$1" in
        --zip) ZIP=1 ;;
        --arch) ARCH=${2:-}; shift ;;
        --releases) RELEASES=${2:-}; shift ;;
        *) echo "usage: $0 [--arch x64|arm64] [--zip] [--releases DIR]" >&2; exit 2 ;;
    esac
    shift
done
case "$ARCH" in
    x64) TARGET=x86_64-pc-windows-msvc ;;
    arm64) TARGET=aarch64-pc-windows-msvc ;;
    *) echo "unknown arch '$ARCH', expected x64 or arm64" >&2; exit 2 ;;
esac

export RUSTUP_HOME=$WINTC/rustup CARGO_HOME=$WINTC/cargo
export PATH=$CARGO_HOME/bin:$WINTC/venv/bin:$WINTC/dotnet:$PATH
export DOTNET_ROOT=$WINTC/dotnet DOTNET_CLI_HOME=$WINTC/dotnet-home DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target/windows}
# Keep this machine's paths out of the binary: Rust's panic messages and C's __FILE__ name the
# source files. The last matching prefix wins, so the most specific comes last. cargo-xwin adds
# CL_FLAGS to the target's C flags only, and clang-cl takes clang's own options after /clang:.
REMAP="$HOME=/home $CARGO_HOME/registry/src=/cargo $CARGO_HOME/git/checkouts=/cargo-git $ROOT=/navia"
RUST_REMAP="" C_REMAP=""
for m in $REMAP; do
    RUST_REMAP="$RUST_REMAP --remap-path-prefix=$m"
    C_REMAP="$C_REMAP /clang:-ffile-prefix-map=$m"
done
export CL_FLAGS="${CL_FLAGS:-}$C_REMAP"
# Link the C runtime statically, so no Visual C++ redistributable is needed.
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static$RUST_REMAP"
export CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static$RUST_REMAP"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/app" "$WORK/out"

# On Windows ARM64, ring compiles its C with plain clang, which can't read the clang-cl
# flags cargo-xwin sets. This clang turns /imsvc into -isystem and drops /clang:. cargo-xwin also
# links its clang-cl to the first clang on PATH, so called by that name it passes everything
# through, and keeping the name keeps clang in clang-cl mode. It stays in $WINTC, since that
# link outlives the build. Don't run two builds at once: they share the link.
if [ "$ARCH" = arm64 ]; then
    mkdir -p "$WINTC/arm64-clang"
    cat > "$WINTC/arm64-clang/clang" <<SHIM
#!/usr/bin/env bash
args=()
while [ \$# -gt 0 ]; do
    if [ "\${0##*/}" != clang ]; then args+=("\$1"); shift
    elif [ "\$1" = /imsvc ]; then args+=(-isystem "\$2"); shift 2
    elif [ "\${1#/clang:}" != "\$1" ]; then args+=("\${1#/clang:}"); shift
    else args+=("\$1"); shift; fi
done
exec -a "\$0" $(command -v clang) "\${args[@]}"
SHIM
    chmod +x "$WINTC/arm64-clang/clang"
    export PATH=$WINTC/arm64-clang:$PATH
fi

cargo xwin build --release --target "$TARGET" -p navia-gpui
cp "$CARGO_TARGET_DIR/$TARGET/release/navia-gpui.exe" "$WORK/app/Navia.exe"
# The same program with a console, for reading its output and crash messages.
python3 -I packaging/windows/console_copy.py "$WORK/app/Navia.exe" "$WORK/app/Navia-console.exe"
cp apps/navia-gpui/assets/fonts/OFL.txt "$WORK/app/Inter-OFL.txt"
cp LICENSE "$WORK/app/LICENSE.txt"

# "[win]" lets vpk pack for Windows on Linux. No delta, since there are no earlier releases here.
"$WINTC/vpk/vpk" "[win]" pack \
    --packId Navia --packTitle Navia --packAuthors Navia --packVersion "$VERSION" \
    --packDir "$WORK/app" --mainExe Navia.exe --runtime "win-$ARCH" --channel "win-$ARCH" \
    --icon apps/navia-gpui/assets/windows/app.ico \
    --splashImage packaging/windows/navia-splash.gif --splashProgressColor "#5A93F5" \
    --shortcuts Desktop,StartMenuRoot --delta None \
    --outputDir "$WORK/out"

mkdir -p "$OUT_DIR"
cp "$WORK/out/Navia-win-$ARCH-Setup.exe" "$OUT_DIR/Navia-Setup-$VERSION-$ARCH.exe"
echo "$OUT_DIR/Navia-Setup-$VERSION-$ARCH.exe"
if [ "$ZIP" = 1 ]; then
    cp "$WORK/out/Navia-win-$ARCH-Portable.zip" "$OUT_DIR/Navia-Portable-$VERSION-$ARCH.zip"
    echo "$OUT_DIR/Navia-Portable-$VERSION-$ARCH.zip"
fi
if [ -n "$RELEASES" ]; then
    mkdir -p "$RELEASES"
    cp "$WORK/out/"* "$RELEASES/"
    ls -l "$RELEASES"
fi
