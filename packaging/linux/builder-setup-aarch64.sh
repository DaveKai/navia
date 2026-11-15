#!/bin/sh
# Adds an arm64 cross toolchain and arm64 libraries to the builder image, still glibc 2.35.
# Ubuntu's snapshot service doesn't carry ports.ubuntu.com, so this uses the live archives,
# for amd64 too, since a library installed for both architectures must be the same version.
set -eu

dpkg --add-architecture arm64
for suite in jammy jammy-updates jammy-security; do
    echo "deb [arch=amd64] http://archive.ubuntu.com/ubuntu $suite main universe"
    echo "deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports $suite main universe"
done > /etc/apt/sources.list
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libc6-dev-arm64-cross \
    libasound2-dev:arm64 libfontconfig-dev:arm64 libfreetype-dev:arm64 libopus-dev:arm64 \
    libwayland-dev:arm64 libxkbcommon-dev:arm64 libzstd-dev:arm64
rm -rf /var/lib/apt/lists/*

rustup target add aarch64-unknown-linux-gnu
chmod -R a+rX /opt/rustup /opt/cargo
