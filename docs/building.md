# Building Navia

The Rust code is a single Cargo workspace. You need stable Rust from [rustup](https://rustup.rs). Releases are
built with 1.99.

## GPUI

The Linux and Windows app is built on GPUI, pinned to one Zed revision. Fetch it and apply Navia's patches with:

```sh
vendor/gpui/fetch-gpui.sh
```

This puts the sources in `vendor/gpui/zed` and `vendor/gpui/wgpu`, which git ignores. Run it again after you
change the revision or a patch. To start clean, delete both folders.

## Linux

Navia runs on Wayland only.

Fedora:

```sh
sudo dnf install gcc clang cmake pkgconf-pkg-config alsa-lib-devel fontconfig-devel freetype-devel \
    opus-devel wayland-devel libxkbcommon-devel vulkan-loader-devel libzstd-devel
```

Arch:

```sh
sudo pacman -S base-devel clang cmake alsa-lib fontconfig freetype2 opus wayland libxkbcommon \
    vulkan-icd-loader zstd
```

Debian and Ubuntu:

```sh
sudo apt install build-essential clang cmake pkg-config libasound2-dev libfontconfig-dev \
    libfreetype-dev libopus-dev libwayland-dev libxkbcommon-dev libvulkan-dev libzstd-dev
```

Build and run:

```sh
vendor/gpui/fetch-gpui.sh
cargo run --release -p navia-gpui
```

`apps/navia-gpui/run.sh` does the same, and also installs a desktop entry so the window gets Navia's icon.

Your login is stored through the Secret Service API, so you need a keyring such as GNOME Keyring or KWallet.

### Environment variables

| Variable | What it does |
| --- | --- |
| `NAVIA_DEV=1` | Signs in from `NAVIDROME_URL`, `NAVIDROME_USER` and `NAVIDROME_PASSWORD`. `run.sh` reads them from `.env.local`. |
| `NAVIA_READ_ONLY=1` | Never writes to the server: no scrobbles, stars, queue saves or playlist edits. |
| `NAVIA_DATA_DIR` | Stores the library, caches and downloads in another folder. |
| `NAVIA_SAMPLE=1` | Runs without a server. Set `NAVIA_SAMPLE_DB` to a Navidrome database to read it, otherwise it makes up data. |

## macOS

You need macOS 26 or later, Xcode with Swift 6, and the Apple Rust targets:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Build the core and copy it into the app:

```sh
apps/navia-apple/scripts/build-core.sh
```

This builds for Apple Silicon. For a universal build, run `crates/navia-ffi/build-xcframework.sh macos` instead.

Then open `apps/navia-apple/Navia.xcodeproj` in Xcode, pick your team under Signing & Capabilities, and build.
A free Personal Team works. From the command line:

```sh
cd apps/navia-apple
xcodebuild -project Navia.xcodeproj -scheme Navia -configuration Debug \
    -derivedDataPath .build DEVELOPMENT_TEAM=<your team ID> build
```

`apps/navia-apple/scripts/mac.sh` builds on a Mac over SSH from Linux. It reads `NAVIA_MAC_HOST`, `NAVIA_MAC_DIR`
and `NAVIA_DEVELOPMENT_TEAM`.

## Windows

Windows builds are cross-compiled from Linux with [cargo-xwin](https://github.com/rust-cross/cargo-xwin). You
also need `clang-cl`, `lld-link` and `llvm-rc` from LLVM, and `cmake`.

```sh
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
vendor/gpui/fetch-gpui.sh
cargo xwin build --release --target x86_64-pc-windows-msvc -p navia-gpui
```

`packaging/windows/build-installer.sh` builds the installer with [Velopack](https://velopack.io). The top of the
script lists what it needs. `--arch arm64` builds for Windows on ARM, with the `aarch64-pc-windows-msvc` target.

## Tests

```sh
cargo test -p navia-core
cargo test -p navia-integrations
cargo test -p navia-gpui
```

Most tests run offline. A few run against a real server when the `NAVIDROME_*` variables are set, and only read
from it.

`crates/navia-core/examples/` has developer tools for checking gapless playback, output devices and
recommendations. Each one explains itself at the top.

## Translations

```sh
python3 -I tools/i18n.py check     # all languages have the same keys and placeholders
python3 -I tools/i18n.py extract   # rebuild locales/en.json from the code
python3 -I tools/i18n.py sync      # copy the translations into the Mac app
```

## Icons

`python3 -I tools/brand_icons.py` regenerates all icons from `assets/brand/`. It needs `rsvg-convert`, Pillow,
NumPy and fontTools. Add `check` to list icons that are out of date.

## Linux packages

`packaging/build-release.sh` builds the `.rpm`, Arch package, `.deb` and tarball in a Docker container, for x86_64
or, with `--arch aarch64`, 64-bit ARM.
`packaging/test-install.sh` tests each one in a clean container. See [packaging/README.md](../packaging/README.md).
