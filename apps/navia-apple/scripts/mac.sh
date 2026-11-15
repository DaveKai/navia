#!/usr/bin/env bash
# Sync this repo to a Mac over SSH and build there, for developing on Linux with a Mac as the build host.
# NAVIA_MAC_HOST (default `mac`) is the SSH host and NAVIA_MAC_DIR (default `Projects/Navia`) the folder there.
# NAVIA_DEVELOPMENT_TEAM, if set, is passed to xcodebuild as DEVELOPMENT_TEAM for signing.
# Builds run through scripts/gui-session.sh, because codesign can't reach the signing key from an SSH session.
#   scripts/mac.sh sync | build | run [extra app args...] | stop
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
REMOTE="${NAVIA_MAC_HOST:-mac}"
REMOTE_DIR="${NAVIA_MAC_DIR:-Projects/Navia}"
DERIVED="$REMOTE_DIR/apps/navia-apple/.build"
APP="$DERIVED/Build/Products/Debug/Navia.app"

sync() {
  rsync -a --delete --exclude 'target/' --exclude 'node_modules/' --exclude '.env*' --exclude '.git/' --exclude 'apps/navia-apple/.build/' --exclude '.DS_Store' "$ROOT/" "$REMOTE:$REMOTE_DIR/"
}

# The version from the workspace Cargo.toml, stamped into Info.plist with the build number and commit.
STAMP="MARKETING_VERSION=$("$ROOT/tools/version.sh") CURRENT_PROJECT_VERSION=$("$ROOT/tools/version.sh" build) NAVIA_COMMIT=$("$ROOT/tools/version.sh" commit)${NAVIA_DEVELOPMENT_TEAM:+ DEVELOPMENT_TEAM=$NAVIA_DEVELOPMENT_TEAM}"

case "${1:-}" in
  sync) sync ;;
  build) sync; ssh "$REMOTE" "cd '$REMOTE_DIR/apps/navia-apple' && mkdir -p .build && scripts/gui-session.sh xcodebuild -quiet -project Navia.xcodeproj -scheme Navia -configuration Debug -derivedDataPath .build $STAMP build > .build/last-build.log 2>&1; rc=\$?; grep -E 'error:|warning: unre|BUILD' .build/last-build.log | cut -c1-300 | sort -u | head -40; exit \$rc" ;;
  # -g keeps the app in the background so it doesn't take focus.
  run) shift; ssh "$REMOTE" "open -g -n '$APP' --args $*" ;;
  stop) ssh "$REMOTE" "pkill -x Navia || true" ;;
  *) echo "usage: $0 sync|build|run|stop"; exit 2 ;;
esac
