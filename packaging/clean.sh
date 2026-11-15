#!/usr/bin/env bash
# Removes this project's packaging containers, images and build output, and nothing else.
# Each is named exactly and nothing is pruned, since Docker may run unrelated services.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

for c in $(docker ps -a --filter name='^navia-pkg-' --format '{{.Names}}'); do
    docker rm -f "$c"
done
# The builder and the base images only this packaging pulls. Not forced, so images in use stay.
for i in navia-pkg-builder-aarch64:local navia-pkg-builder:local \
    ubuntu:22.04 ubuntu:24.04 debian:12 debian:13 fedora:latest archlinux:latest archlinux:base-devel \
    goreleaser/nfpm:v2.48.0 \
    ubuntu@sha256:076e7bf82319619f28dd0aa9c02ab34b055ad37100395b4f49a2ffba04fb5e11 \
    goreleaser/nfpm@sha256:d57a1452e7a019896b355f7b1ec67262278f6c503631f652d4fce7aef9ac8df7; do
    docker image inspect "$i" >/dev/null 2>&1 && docker rmi "$i"
done
rm -rf "$ROOT/target/package" "$ROOT/packaging/dist"
