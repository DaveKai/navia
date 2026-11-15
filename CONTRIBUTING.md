# Contributing to Navia

Bug reports, fixes, translations and features are all welcome.

## Setup

1. Install Rust and the dependencies for your platform from [docs/building.md](docs/building.md).
2. Fetch GPUI:

   ```sh
   vendor/gpui/fetch-gpui.sh
   ```

3. Test against your own Navidrome server with a small library. See
   [Navidrome's install guide](https://www.navidrome.org/docs/installation/). If other people use the server, run
   Navia with `NAVIA_READ_ONLY=1`.

## Building

- **Linux**: `cargo run --release -p navia-gpui`
- **macOS**: run `apps/navia-apple/scripts/build-core.sh`, then build `apps/navia-apple/Navia.xcodeproj` in Xcode
  with your own team.
- **Windows**: `cargo xwin build --release --target x86_64-pc-windows-msvc -p navia-gpui` from Linux.

Before you open a pull request, run:

```sh
cargo test -p navia-core
cargo test -p navia-integrations
cargo build -p navia-gpui
cargo xwin check --target x86_64-pc-windows-msvc -p navia-gpui
python3 -I tools/i18n.py check
```

If you couldn't build a platform, say which one in the pull request.

## Code

- Shared logic goes in `crates/navia-core`. The apps only draw the interface.
- Colors, sizes, radii, shadows, motion and fonts come from the theme tokens (`apps/navia-gpui/src/theme.rs`,
  `apps/navia-apple/Navia/Theme/Tokens.swift`) and the shared components. Don't hard-code values in a view. Add a
  token, or a variant of the shared component.
- Change both apps together, with the same token and component names.
- Only show what the server's data supports.
- Don't assume the server has a setting, plugin or library size. Check what it supports and work without it.
- Comments explain why, not what.
- Match the style of the code around you. The Rust code isn't run through `cargo fmt`, so don't reformat files.
  Don't add warnings.
- Remove code your change leaves unused.

## Pull requests

- One topic per pull request.
- Commit messages use a type and an optional scope, then a short description in the imperative:
  `fix(player): keep the queue position after a skip`, `feat(airplay): add volume sync`, `docs: update building`.
  Types are `feat`, `fix`, `perf`, `refactor`, `docs`, `test` and `chore`. Scopes are the part of the app, such as
  `core`, `player`, `autoplay`, `library`, `ui`, `mac`, `linux`, `windows`, `airplay` or `i18n`.
- Say how you tested it: platforms, server version, and measurements for audio changes such as gapless,
  crossfade or seeking.
- Add before and after screenshots for visible changes.
- Contributions are licensed under GPL-3.0-or-later.

## Bug reports

Include:

- your OS and version, and on Linux your distribution and desktop
- Navia's version, from Settings › About
- your server and its version
- what you did, what you expected, what happened, and logs if you have them

Leave out passwords, tokens and private server addresses.

For feature requests, say what you want to do and which server data it would use.

## Translations

The French, Spanish and German translations were machine-translated and haven't been reviewed by a native speaker yet. Corrections are very welcome.

The translations are in `locales/`, shared by both apps. The keys are the English text. Plurals use the CLDR
categories (`one`, `other`, and `many` where a language has it). Keep `{placeholders}` as they are.

- To fix a translation, edit `locales/<lang>.json`.
- To add a language, copy every key from `en.json` into `locales/<lang>.json` and add the language to `LANGS` in
  `tools/i18n.py`.
- Then run `python3 -I tools/i18n.py sync` and `python3 -I tools/i18n.py check`.

## Design

For a visible change, open an issue with a sketch or screenshots first. Build it on both platforms from the shared
tokens and components. A new value goes in both `theme.rs` and `Tokens.swift`.

Icons are generated from `assets/brand/` by `tools/brand_icons.py`. Edit the sources and run it, not the generated
files.
