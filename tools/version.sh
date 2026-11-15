#!/usr/bin/env bash
# Prints Navia's version, read from [workspace.package] in the workspace Cargo.toml.
#   tools/version.sh          the version, such as 0.1.0
#   tools/version.sh commit   the short commit of a test build, empty for a release (HEAD tagged v<version>)
#   tools/version.sh full     "0.1.0+543" for a test build (543 is the commit count), "0.1.0" for a release
#   tools/version.sh display  what the apps show, such as "0.1.0+543 (f6e8534)"
#   tools/version.sh build    a build number that only grows, the number of commits up to HEAD
#   tools/version.sh xcode    the Mac's xcodebuild version settings, with no commit for a release
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
commit() {
    if git -C "$ROOT" describe --exact-match --tags --match "v$VERSION" HEAD >/dev/null 2>&1; then
        echo ""
    else
        git -C "$ROOT" rev-parse --short=7 HEAD 2>/dev/null || echo ""
    fi
}
case "${1:-version}" in
    version) echo "$VERSION" ;;
    commit) commit ;;
    full) c="$(commit)"; if [ -n "$c" ]; then echo "$VERSION+$(git -C "$ROOT" rev-list --count HEAD)"; else echo "$VERSION"; fi ;;
    display) c="$(commit)"; if [ -n "$c" ]; then echo "$VERSION+$(git -C "$ROOT" rev-list --count HEAD) ($c)"; else echo "$VERSION"; fi ;;
    build) git -C "$ROOT" rev-list --count HEAD ;;
    xcode) echo "MARKETING_VERSION=$VERSION CURRENT_PROJECT_VERSION=$(git -C "$ROOT" rev-list --count HEAD) NAVIA_COMMIT=$(commit)" ;;
    *) echo "usage: $0 [version|commit|full|display|build|xcode]" >&2; exit 2 ;;
esac
