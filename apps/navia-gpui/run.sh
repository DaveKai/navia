#!/usr/bin/env bash
# Runs the app signed in with the saved account, like a normal install.
#
# NAVIA_DEV=1 takes the login from .env.local (NAVIDROME_URL, NAVIDROME_USER, NAVIDROME_PASSWORD) and turns off Change
# Server and Sign Out. NAVIA_REPORT_PLAYS and NAVIA_DISCORD force reporting on for tests.
set -eu
cd "$(dirname "$0")/../.."
if [ "${NAVIA_DEV:-}" = 1 ] && [ -f ./.env.local ]; then set -a; . ./.env.local; set +a; fi
export CARGO_TARGET_DIR="$PWD/target/gpui-core"
# The desktop matches the window to $APP_ID.desktop for its icon, and a build in target/ has none installed.
packaging/dev-desktop-entry.sh || true
exec cargo run --release -p navia-gpui
