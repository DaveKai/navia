#!/usr/bin/env bash
# Builds the Linux release packages in containers: .deb and .rpm through nFPM, and an Arch package.
# Output goes to packaging/dist/<arch>/. Needs only Docker and rsync on the host.
#
#   packaging/build-release.sh              build the binary, then package it
#   packaging/build-release.sh --binary     only build the binary
#   packaging/build-release.sh --package    only package the binary already built
#   --arch aarch64                          build for 64-bit ARM instead, cross-compiled
#
# Every container and image here is named navia-pkg-*, and packaging/clean.sh removes them.
set -euo pipefail

BASE_IMAGE=ubuntu:22.04@sha256:076e7bf82319619f28dd0aa9c02ab34b055ad37100395b4f49a2ffba04fb5e11
APT_SNAPSHOT=20261101T000000Z
RUST_VERSION=1.99.1
NFPM_IMAGE=goreleaser/nfpm:v2.48.0@sha256:d57a1452e7a019896b355f7b1ec67262278f6c503631f652d4fce7aef9ac8df7
ARCH_IMAGE=archlinux:base-devel
BUILDER=navia-pkg-builder:local
BUILDER_AARCH64=navia-pkg-builder-aarch64:local

ARCH=x86_64
STEP=all
while [ $# -gt 0 ]; do
    case "$1" in
        --binary|--package) STEP="$1" ;;
        --arch) ARCH="${2:-}"; shift ;;
        *) echo "usage: $0 [--arch x86_64|aarch64] [--binary|--package]" >&2; exit 2 ;;
    esac
    shift
done
# nFPM takes Go's architecture names.
case "$ARCH" in
    x86_64) NFPM_ARCH=amd64 ;;
    aarch64) NFPM_ARCH=arm64 ;;
    *) echo "unknown arch '$ARCH', expected x86_64 or aarch64" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/packaging"
WORK="$ROOT/target/package"
STAGE="$WORK/stage"
DIST="$PKG/dist/$ARCH"
CARGO_TARGET="$WORK/cargo-target"
[ "$ARCH" = x86_64 ] || CARGO_TARGET="$WORK/cargo-target-$ARCH"
OWNER="$(id -u):$(id -g)"

# shellcheck source=app.env
source "$PKG/app.env"
VERSION="$("$ROOT/tools/version.sh")"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct)}"

# The window's app ID must match the desktop file's name, or desktops show a generic icon.
CODE_APP_ID="$(sed -n 's/^pub const APP_ID: &str = "\([^"]*\)";/\1/p' "$ROOT/crates/navia-core/src/client/mod.rs" | head -1)"
if [ "$CODE_APP_ID" != "$APP_ID" ]; then
    echo "APP_ID in crates/navia-core/src/client/mod.rs is '$CODE_APP_ID' but packaging/app.env says '$APP_ID'" >&2
    exit 1
fi

build_binary() {
    if ! docker image inspect "$BUILDER" >/dev/null 2>&1; then
        docker rm -f navia-pkg-setup >/dev/null 2>&1 || true
        docker run --name navia-pkg-setup -v "$PKG/linux/builder-setup.sh:/setup.sh:ro" \
            "$BASE_IMAGE" sh /setup.sh "$APT_SNAPSHOT" "$RUST_VERSION"
        docker commit -c 'ENV RUSTUP_HOME=/opt/rustup' -c 'ENV CARGO_HOME=/opt/cargo' \
            -c 'ENV PATH=/opt/cargo/bin:/usr/bin:/bin' navia-pkg-setup "$BUILDER" >/dev/null
        docker rm navia-pkg-setup >/dev/null
    fi
    # The ARM builder adds a cross toolchain and arm64 libraries to the one above.
    local image="$BUILDER" cross=()
    if [ "$ARCH" = aarch64 ]; then
        if ! docker image inspect "$BUILDER_AARCH64" >/dev/null 2>&1; then
            docker rm -f navia-pkg-setup >/dev/null 2>&1 || true
            docker run --name navia-pkg-setup -v "$PKG/linux/builder-setup-aarch64.sh:/setup.sh:ro" \
                "$BUILDER" sh /setup.sh
            docker commit navia-pkg-setup "$BUILDER_AARCH64" >/dev/null
            docker rm navia-pkg-setup >/dev/null
        fi
        image="$BUILDER_AARCH64"
        cross=(-e CARGO_BUILD_TARGET=aarch64-unknown-linux-gnu
            -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
            -e CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
            -e CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
            -e AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
            -e PKG_CONFIG_ALLOW_CROSS=1
            -e PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig)
    fi

    # Build from a private copy, so edits during a long build can't produce a mixed binary.
    mkdir -p "$WORK/src" "$WORK/cargo-home" "$CARGO_TARGET"
    rsync -a --delete --exclude target --exclude node_modules --exclude .git --exclude '.env*' \
        --exclude apps/navia-apple --exclude packaging/dist "$ROOT/" "$WORK/src/"

    # Reuse what this machine already downloaded. It's copied, never shared with the container.
    for d in registry git; do
        if [ ! -e "$WORK/cargo-home/$d" ] && [ -d "$HOME/.cargo/$d" ]; then
            cp -a "$HOME/.cargo/$d" "$WORK/cargo-home/$d"
        fi
    done

    # Fixed paths and remapped prefixes keep the build machine out of the binary. Runs as the
    # calling user, so nothing in the tree ends up owned by root.
    docker run --rm --name navia-pkg-build --user "$OWNER" \
        --memory "${NAVIA_PKG_MEMORY:-14g}" --memory-swap "${NAVIA_PKG_MEMORY:-14g}" \
        -v "$WORK/src:/build/navia:ro" \
        -v "$WORK/cargo-home:/build/cargo-home" \
        -v "$CARGO_TARGET:/build/target" \
        -e HOME=/tmp \
        -e CARGO_HOME=/build/cargo-home \
        -e CARGO_TARGET_DIR=/build/target \
        -e CARGO_PROFILE_RELEASE_STRIP=symbols \
        -e CARGO_PROFILE_RELEASE_DEBUG=0 \
        -e CARGO_INCREMENTAL=0 \
        -e SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
        -e NAVIA_PACKAGE_NAME="$APP_NAME" \
        -e RUSTFLAGS="--remap-path-prefix=/build/navia=/navia --remap-path-prefix=/build/cargo-home=/cargo" \
        "${cross[@]}" \
        -w /build/navia "$image" \
        nice -n 15 cargo build --release --locked -j "${NAVIA_PKG_JOBS:-8}" -p navia-gpui
}

render() { # template, output
    sed -e "s|@APP_NAME@|$APP_NAME|g" \
        -e "s|@APP_ID@|$APP_ID|g" \
        -e "s|@APP_DISPLAY_NAME@|$APP_DISPLAY_NAME|g" \
        -e "s|@APP_SUMMARY@|$APP_SUMMARY|g" \
        -e "s|@APP_MAINTAINER@|$APP_MAINTAINER|g" \
        -e "s|@VERSION@|$VERSION|g" \
        -e "s|@ARCH@|$ARCH|g" \
        -e "s|@NFPM_ARCH@|$NFPM_ARCH|g" \
        -e "s|@SHA256@|${TARBALL_SHA256:-}|g" \
        "$1" > "$2"
}

package() {
    local binary="$CARGO_TARGET/release/navia-gpui"
    [ "$ARCH" = x86_64 ] || binary="$CARGO_TARGET/$ARCH-unknown-linux-gnu/release/navia-gpui"
    [ -x "$binary" ] || { echo "no binary at $binary" >&2; exit 1; }

    rm -rf "$STAGE" "$DIST"
    mkdir -p "$STAGE" "$DIST"
    install -m 755 "$binary" "$STAGE/$APP_NAME"
    # The hicolor tree as it lands in /usr/share/icons, generated by tools/brand_icons.py.
    install -Dm 644 "$PKG/linux/icons/app.svg" "$STAGE/icons/hicolor/scalable/apps/$APP_ID.svg"
    local png size
    for png in "$PKG"/linux/icons/app-*.png; do
        size="${png##*/app-}"; size="${size%.png}"
        install -Dm 644 "$png" "$STAGE/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
    done
    install -m 644 "$ROOT/LICENSE" "$STAGE/LICENSE"
    install -m 644 "$PKG/linux/$APP_ID.metainfo.xml" "$STAGE/$APP_ID.metainfo.xml"
    render "$PKG/linux/app.desktop.in" "$STAGE/$APP_ID.desktop"
    chmod 644 "$STAGE/$APP_ID.desktop"
    desktop_check "$STAGE/$APP_ID.desktop"

    # Paths in the nFPM config are relative to the stage directory.
    render "$PKG/linux/nfpm.yaml.in" "$STAGE/nfpm.yaml"
    icons_check "$STAGE/nfpm.yaml"
    for packager in deb rpm; do
        docker run --rm --name "navia-pkg-nfpm-$packager" --user "$OWNER" \
            -e SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
            -v "$STAGE:/stage:ro" -v "$DIST:/dist" -w /stage \
            "$NFPM_IMAGE" package --config nfpm.yaml --packager "$packager" --target /dist/
    done

    # The Arch package uses the same binary the other distros were tested with.
    local tarball="$APP_NAME-$VERSION-$ARCH.tar.gz"
    (cd "$STAGE" && tar --sort=name --owner=0 --group=0 --numeric-owner \
        --mtime="@$SOURCE_DATE_EPOCH" -cf - \
        "$APP_NAME" icons "$APP_ID.desktop" "$APP_ID.metainfo.xml" LICENSE) \
        | gzip -n > "$DIST/$tarball"
    TARBALL_SHA256="$(sha256sum "$DIST/$tarball" | cut -d' ' -f1)"
    render "$PKG/linux/PKGBUILD.in" "$DIST/PKGBUILD"

    # makepkg refuses to run as root. -d because nothing here needs the runtime libraries.
    # CARCH names the package's architecture; nothing is compiled, so x86_64 Arch can pack ARM.
    docker run --rm --name navia-pkg-makepkg \
        -e SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" -e OWNER="$OWNER" -e CARCH="$ARCH" \
        -v "$DIST:/dist" "$ARCH_IMAGE" bash -euc '
            sed -i "s/^CARCH=.*/CARCH=\"$CARCH\"/" /etc/makepkg.conf
            useradd -m packager
            cp -r /dist /home/packager/build
            chown -R packager /home/packager/build
            su packager -c "cd ~/build && PKGEXT=.pkg.tar.zst makepkg -d -f --noconfirm"
            cp /home/packager/build/*.pkg.tar.zst /dist/
            chown "$OWNER" /dist/*.pkg.tar.zst
        '

    (cd "$DIST" && sha256sum ./*.deb ./*.rpm ./*.pkg.tar.zst ./*.tar.gz PKGBUILD > SHA256SUMS)
    ls -l "$DIST"
}

# Catches a broken desktop entry at build time.
desktop_check() {
    grep -q "^Icon=$APP_ID$" "$1"
    grep -q "^StartupWMClass=$APP_ID$" "$1"
    grep -q "^Exec=$APP_NAME" "$1"
}

# nfpm.yaml lists each icon, so a size added to tools/brand_icons.py must be listed there too.
icons_check() {
    local f
    for f in $(cd "$STAGE" && find icons -type f | sort); do
        grep -q "src: ./$f$" "$1" || { echo "$f is staged but not in nfpm.yaml.in" >&2; exit 1; }
    done
}

case "$STEP" in
    --binary) build_binary ;;
    --package) package ;;
    all) build_binary; package ;;
esac
