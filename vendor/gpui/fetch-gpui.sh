#!/usr/bin/env bash
# Downloads the exact GPUI source Navia pins into vendor/gpui/zed and vendor/gpui/wgpu, without
# cloning Zed's full history (a `git` dependency needs a multi-gigabyte clone; tarballs of one
# revision are enough to build and pin), then applies Navia's patches. Re-run after changing a revision.
set -euo pipefail
cd "$(dirname "$0")"

ZED_REV=d2a4fb6ded7c470be25d1541281da3882495b65b
WGPU_REV=cddb2f5d80dc0009b2ef6767635140cacf90a007

fetch() { # owner/repo rev dest
    if [ ! -d "$3" ]; then
        mkdir -p "$3.part"
        curl -fsSL "https://codeload.github.com/$1/tar.gz/$2" | tar -xz --strip-components=1 -C "$3.part"
        mv "$3.part" "$3"
    fi
}

fetch zed-industries/zed "$ZED_REV" zed
fetch gfx-rs/wgpu "$WGPU_REV" wgpu

# Point Zed's git dependencies on wgpu at the vendored tree, so Cargo never clones it.
sed -i -E "s#wgpu = \{ git = \"https://github.com/gfx-rs/wgpu\", rev = \"$WGPU_REV\"#wgpu = { path = \"../wgpu/wgpu\"#" zed/Cargo.toml
sed -i -E "s#naga = \{ git = \"https://github.com/gfx-rs/wgpu\", rev = \"$WGPU_REV\"#naga = { path = \"../../../wgpu/naga\"#" zed/crates/gpui_wgpu/Cargo.toml
# Local fixes to the vendored GPUI that can't be expressed in app code. Safe to re-run: a patch
# that's already applied is skipped.
for p in patches/*.patch; do
    patch -d zed -p1 -N --forward --silent -r - < "$p" || true
done
echo "vendored zed $ZED_REV and wgpu $WGPU_REV"
