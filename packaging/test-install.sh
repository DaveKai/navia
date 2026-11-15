#!/usr/bin/env bash
# Installs each package from packaging/dist/<arch> in a throwaway container and checks that
# dependencies resolve, the binary runs, and uninstalling leaves nothing behind.
#
#   packaging/test-install.sh [x86_64|aarch64]
#
# aarch64 needs QEMU registered with binfmt_misc on an x86_64 host, and skips Arch, which has
# no official arm64 image.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="${1:-x86_64}"
DIST="$ROOT/packaging/dist/$ARCH"
case "$ARCH" in
    x86_64) PLATFORM=linux/amd64 ;;
    aarch64) PLATFORM=linux/arm64 ;;
    *) echo "usage: $0 [x86_64|aarch64]" >&2; exit 2 ;;
esac
source "$ROOT/packaging/app.env"

# Run inside the container, after the package is installed.
CHECK='
set -e
b=/usr/bin/'"$APP_NAME"'
if ldd "$b" | grep "not found"; then exit 11; fi
"$b" --version
for l in libvulkan.so.1 libwayland-client.so.0 libwayland-egl.so.1; do
    ldconfig -p | grep -q "$l" || { echo "runtime library missing: $l"; exit 12; }
done
test -f /usr/share/applications/'"$APP_ID"'.desktop
test -f /usr/share/icons/hicolor/scalable/apps/'"$APP_ID"'.svg
test -f /usr/share/icons/hicolor/48x48/apps/'"$APP_ID"'.png
test -f /usr/share/metainfo/'"$APP_ID"'.metainfo.xml
'
GONE='
for f in /usr/bin/'"$APP_NAME"' /usr/share/applications/'"$APP_ID"'.desktop /usr/share/icons/hicolor/scalable/apps/'"$APP_ID"'.svg; do
    if [ -e "$f" ]; then echo "left behind: $f"; exit 13; fi
done
'
deb_steps="apt-get update -qq && apt-get install -y -qq --no-install-recommends /dist/${APP_NAME}_*.deb && $CHECK apt-get purge -y -qq $APP_NAME && $GONE"
rpm_steps="dnf install -y -q /dist/${APP_NAME}-*.rpm && $CHECK dnf remove -y -q $APP_NAME && $GONE"
arch_steps="pacman -Sy --noconfirm --quiet && pacman -U --noconfirm /dist/${APP_NAME}-bin-*.pkg.tar.zst && $CHECK pacman -Rns --noconfirm ${APP_NAME}-bin && $GONE"

failed=0
run() { # label, image, steps
    local log="$ROOT/target/package/test-$ARCH-$1.log"
    if docker run --rm --name "navia-pkg-test-$1" --platform "$PLATFORM" -v "$DIST:/dist:ro" "$2" bash -c "$3" > "$log" 2>&1; then
        echo "PASS $1 ($2, $ARCH): $(grep -m1 "^$APP_NAME " "$log")"
    else
        echo "FAIL $1 ($2, $ARCH), see $log"
        failed=1
    fi
}

mkdir -p "$ROOT/target/package"
run ubuntu-2204 ubuntu:22.04 "$deb_steps"
run ubuntu-2404 ubuntu:24.04 "$deb_steps"
run debian-12 debian:12 "$deb_steps"
run debian-13 debian:13 "$deb_steps"
run fedora fedora:latest "$rpm_steps"
[ "$ARCH" = x86_64 ] && run arch archlinux:latest "$arch_steps"
exit "$failed"
