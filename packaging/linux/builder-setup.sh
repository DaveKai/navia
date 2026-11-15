#!/bin/sh
# Prepares the build environment in the pinned Ubuntu 22.04 image (glibc 2.35).
# build-release.sh commits the result, so the build leaves no BuildKit cache behind.
set -eu
APT_SNAPSHOT="$1"
RUST_VERSION="$2"

# The snapshot mirror is https-only, and the base image has no CA certificates yet.
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates

for suite in jammy jammy-updates jammy-security; do
    echo "deb https://snapshot.ubuntu.com/ubuntu/$APT_SNAPSHOT $suite main universe"
done > /etc/apt/sources.list
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential clang cmake curl git ca-certificates perl pkg-config \
    libasound2-dev libfontconfig-dev libfreetype-dev libopus-dev \
    libwayland-dev libxkbcommon-dev \
    libvulkan1 libzstd-dev
rm -rf /var/lib/apt/lists/*

export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
curl -fsSL https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain "$RUST_VERSION"
rm /tmp/rustup-init.sh
chmod -R a+rX /opt/rustup /opt/cargo
